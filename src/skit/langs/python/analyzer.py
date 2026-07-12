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

    # "const" | "input" — the source-anchor axis (field-aligned with ParamDecl.binding).
    binding: str
    name: str  # const: variable name; input: display name (input-1, input-2, …)
    type: str = "str"  # one of INJECTABLE_TYPES
    default: str | int | float | bool | None = None  # const: the original value in the source
    prompt: str = ""  # input: the literal prompt of input() (if any)
    order: int = -1  # input: which input() call (0-based); -1 for const
    lineno: int = 0
    secret: bool = False  # heuristic pre-check, editable during onboarding
    # Demotion signal (UX spec §0): a candidate that *parses* as a constant but whose usage
    # says "not a parameter" — currently only "accumulator" (literal init + AugAssign anywhere,
    # or reassigned inside a loop body). Demoted candidates default to unchecked at onboarding,
    # with the reason surfaced; clean candidates default to checked.
    demoted: bool = False
    demotion: str = ""  # symbolic reason id; the UI owns the human wording


@dataclass
class Analysis:
    candidates: list[Candidate] = field(default_factory=list)
    frameworks: list[str] = field(default_factory=list)  # detected CLI frameworks
    syntax_error: bool = False
    uses_argv: bool = False  # sys.argv appears -> the run form gets a passthrough-args hint
    # Filename-looking string literals passed directly as call arguments (never bound to a
    # name): the "extract this into a named constant to manage it" hint. Capped, deduped,
    # source order. Only literals a cheap deterministic rule can vouch for — nothing else
    # (see the 'RGB' exclusion in the UX spec: no domain-knowledge guesses).
    filename_literals: list[str] = field(default_factory=list)

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
            secret=_is_secret_name(name),
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
        prompt = _literal_prompt(call)
        candidate = Candidate(
            binding="input",
            name=f"input-{i + 1}",
            prompt=prompt,
            order=i,
            lineno=call.lineno,
            secret=_is_secret_name(prompt),
        )
        candidate.type = "str"  # pragma: no mutate — matches Candidate's own field default
        out.append(candidate)
    return out


def _match_inputs(
    stored: list[tuple[int, str]], current: list[tuple[int, str]]
) -> dict[int, tuple[int, bool]]:
    """Bind each stored input (its recorded ``order``, ``prompt``) to a call site in the CURRENT
    source (3a). Shared by reconcile (drift detection) and shim (actual injection) so both agree on
    exactly the same call site for a given definition -- the reason this lives in analyzer rather
    than in either caller (A2).

    A value must follow its *question*, not its position: keying purely by ``order`` (B1's original
    design) breaks the instant a source edit inserts or deletes an *earlier* input() call, silently
    shifting every later position -- a secret-marked definition can then attach to a different
    prompt with no warning at all. So the literal prompt text is tried first (it survives a shift);
    bare position is only a fallback, and is trusted as "no news" only when neither side has a
    prompt to compare in the first place (a dynamic/absent prompt, or a spec stored before 3a) --
    that case is no worse than the pre-3a behaviour, so it must not manufacture a new warning.

    Returns ``{stored_order: (current_order, ambiguous)}``. A stored order absent from the result
    could not be matched at all (genuinely gone -- the caller reports it as missing). ``ambiguous``
    is True when position had to be trusted *despite* having a prompt to check -- either the prompt
    no longer appears anywhere (likely edited/renamed) or it now matches more than one call site (two
    prompts collide) -- both are exactly the silent-rebind risk this function exists to surface, so
    callers must turn it into a visible warning rather than silently treating it as "ok".

    Two passes: exact prompt matches are resolved first and their current-order claimed, so a
    *different* stored entry's positional fallback can never be handed a call site some other
    definition already owns by an exact prompt match -- e.g. deleting input #1 entirely (its prompt
    now matches nothing) must not let it fall back onto position 0, when input #2's own prompt has
    already, and correctly, claimed position 0 for itself. Without this, the deleted entry would
    silently "recover" a value onto a call site someone else already owns.

    The exact pass itself must also be 1:1, not just enforced against the fallback pass: two or more
    STORED entries can legitimately share the identical literal prompt (a retry pattern like two
    `input("Go? ")` calls, both managed). If the current source now has exactly one call site with
    that prompt (the user deleted one of the two calls), every one of those stored entries would
    otherwise resolve its *own* "unique candidate" check independently and all exact-match onto the
    same current order -- silently binding two different definitions to one call site. Downstream,
    reconcile would call all of them "ok" (no warning at all) and shim would splice two replacements
    over the same `input` callee span, corrupting the injected copy into unparsable source. So the
    exact pass claims its current-order as it goes: the first stored entry (in the order given) that
    uniquely resolves a prompt wins that current order outright, and any later stored entry whose own
    "unique" candidate has *already* been claimed loses the exact match and falls through to the
    positional-fallback pass below -- where it is correctly reported ``missing`` (its bare position no
    longer exists either) or flagged ``ambiguous`` (a different call now sits at that position), but
    never silently double-bound.
    """
    current_by_order = dict(current)
    by_prompt: dict[str, list[int]] = {}
    for order, prompt in current:
        if prompt:
            by_prompt.setdefault(prompt, []).append(order)

    exact: dict[int, int] = {}
    claimed: set[int] = set()
    _match_prompt_multisets(stored, by_prompt, exact, claimed)
    for order, prompt in stored:
        if order in exact:
            continue
        if prompt:
            candidates = by_prompt.get(prompt, [])
            if len(candidates) == 1 and candidates[0] not in claimed:
                exact[order] = candidates[0]
                claimed.add(candidates[0])

    out: dict[int, tuple[int, bool]] = {}
    for order, prompt in stored:
        if order in exact:
            out[order] = (exact[order], False)
            continue
        # No exact prompt match (no prompt to compare, the prompt matches nothing anymore, it
        # collides across multiple call sites, or its one candidate was already claimed by another
        # stored entry's exact match): fall back to position, but never onto a call site an exact
        # match already claimed, and flag it as ambiguous unless there was never a prompt to check
        # in the first place (not a new risk, see the module-level note above).
        if order in current_by_order and order not in claimed:
            out[order] = (order, bool(prompt))
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


def _match_prompt_multisets(
    stored: list[tuple[int, str]],
    by_prompt: dict[str, list[int]],
    exact: dict[int, int],
    claimed: set[int],
) -> None:
    """Multiset pass: when the stored and current sides have the SAME number of call
    sites for a prompt, pair them in positional order. A retry pattern — two identical
    `input("Go? ")` calls, both managed — is a stable shape, and without this pass the
    per-entry uniqueness rule would flag it as a rebind on every run, forever (resync
    can't fix what isn't drift)."""
    stored_by_prompt: dict[str, list[int]] = {}
    for order, prompt in stored:
        if prompt:
            stored_by_prompt.setdefault(prompt, []).append(order)
    for prompt, stored_orders in stored_by_prompt.items():
        current_orders = by_prompt.get(prompt, [])
        if len(stored_orders) > 1 and len(current_orders) == len(stored_orders):
            for stored_order, current_order in zip(
                sorted(stored_orders), sorted(current_orders), strict=True
            ):
                exact[stored_order] = current_order
                claimed.add(current_order)


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
