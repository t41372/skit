"""The mutation gate's blind spot, measured — so it can never quietly grow.

AGENTS.md principle 5 presents mutmut as a hard gate. It is, for the code it can see.
mutmut skips every decorated function and every decorated class (subtree and all), so a
Typer command body, an `@on` handler, an `@override`, and every method of a `@dataclass`
model are verified by coverage alone — and coverage cannot tell a dead branch from a live
one. Design-audit rounds 11 and 12 each shipped a branch that could never fire; both sat
here.

The ratchet below is not a style rule. It is the honest size of the hole, and moving it
UP means the next contributor inherits more unverifiable code, so it may only move up with
a stated reason. Moving it down is always welcome: lift the decision out of the decorated
function into a plain helper, and the helper gets mutated.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

# The REAL tree, even under mutmut's own baseline (which runs this suite inside
# mutants/, where the trampoline rewrite would both inflate the measure and BE the
# blind spot). conftest.real_repo_root strips the mutants/ prefix.
from conftest import real_repo_root

SRC = real_repo_root() / "src" / "skit"

# Measured on the tree this test shipped with. See the module docstring before changing.
MAX_SKIPPED_LINES = 4369
MAX_SKIPPED_SHARE = 0.19


def _analyzed_files() -> list[Path]:
    """Every module mutmut is pointed at (`source_paths = ["src/skit/"]`), minus the
    translation catalogs, which hold no Python."""
    return sorted(p for p in SRC.rglob("*.py") if "locales" not in p.parts)


def _is_bare_static_or_class(decorators: list[ast.expr]) -> bool:
    """mutmut's one exemption: a single bare `@staticmethod`/`@classmethod` (a Name, not a
    Call and not one of several)."""
    if len(decorators) != 1:
        return False
    only = decorators[0]
    return isinstance(only, ast.Name) and only.id in ("staticmethod", "classmethod")


def _measure() -> tuple[int, int, dict[str, int]]:
    """Return (total function-body lines, lines mutmut never mutates, per-decorator split)."""
    total = skipped = 0
    by_decorator: dict[str, int] = {}
    for path in _analyzed_files():
        tree = ast.parse(path.read_text(encoding="utf-8"))
        # A decorated CLASS is pruned whole, so its methods are invisible too — including
        # the bare @staticmethod/@classmethod that would otherwise have been exempt.
        in_skipped_class: set[int] = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.ClassDef) and node.decorator_list:
                in_skipped_class.update(
                    id(sub)
                    for sub in ast.walk(node)
                    if isinstance(sub, (ast.FunctionDef, ast.AsyncFunctionDef))
                )
        for node in ast.walk(tree):
            if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            lines = node.end_lineno - node.lineno + 1  # ty: ignore[unsupported-operator]
            total += lines
            names = [ast.unparse(d).split("(")[0] for d in node.decorator_list]
            if id(node) in in_skipped_class:
                label = names[0] if names else "(method of a decorated class)"
            elif names and not _is_bare_static_or_class(node.decorator_list):
                label = names[0]
            else:
                continue
            skipped += lines
            by_decorator[label] = by_decorator.get(label, 0) + lines
    return total, skipped, by_decorator


def test_mutation_blind_spot_has_not_grown() -> None:
    total, skipped, by_decorator = _measure()
    detail = ", ".join(f"{k}={v}" for k, v in sorted(by_decorator.items(), key=lambda kv: -kv[1]))
    assert skipped <= MAX_SKIPPED_LINES, (
        f"{skipped} function-body lines are now invisible to mutmut (was {MAX_SKIPPED_LINES}); "
        f"{skipped / total:.1%} of {total}. Breakdown: {detail}. Lift the new logic out of "
        "its decorated function into a plain helper, or raise the ratchet with a reason."
    )
    assert skipped / total <= MAX_SKIPPED_SHARE


def test_the_blind_spot_covers_the_cli_and_tui_front_doors() -> None:
    """The number alone would let the shape drift. These are the surfaces it must name."""
    _, _, by_decorator = _measure()
    assert by_decorator["app.command"] > 1000, "Typer command bodies are the largest blind area"
    assert by_decorator["override"] > 500, "Textual's compose/check_action are unmutated"
    assert "on" in by_decorator, "Textual @on handlers are unmutated"


@pytest.mark.parametrize("decorated", [True, False])
def test_mutmut_still_skips_decorated_functions(decorated: bool) -> None:
    """Pin the assumption the ratchet rests on, against the installed mutmut.

    If a future mutmut learns to mutate decorated functions this fails, and the right
    response is to delete this whole file — the hole is gone.
    """
    file_mutation = pytest.importorskip("mutmut.mutation.file_mutation")
    src = "@app.command()\ndef f():\n    return 1\n" if decorated else "def f():\n    return 1\n"
    _mutated_source, mutant_names = file_mutation.mutate_file_contents("probe.py", src)
    assert bool(mutant_names) is not decorated
