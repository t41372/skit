"""Mutation-kill tests for skit/langs/python/analyzer.py.

Three detection details with real add-time consequences:
- filename-literal hints must keep scanning a call's later arguments past a non-string one
  (`continue`, not `break`), or a filename after a non-string positional is missed.
- a constant reassigned inside a loop body (AnnAssign target) is a working variable, not a
  parameter, so the candidate is demoted to unchecked at onboarding.
- a script that binds `input` itself disables input-candidate detection file-wide (the
  `if _bound_names(tree).get("input"): return []` guard), so the shim never rewrites the
  script's OWN function into a stdin-fallback wrapper.
"""

from __future__ import annotations

from skit.langs.python.analyzer import analyze


def test_filename_hint_found_after_a_non_string_argument() -> None:
    """A filename literal that appears *after* a non-string positional arg is still detected: the
    inner arg loop must `continue` past the int, not `break` out of the call (mutant_10)."""
    analysis = analyze("f(1, 'notes.txt')\n")
    # The non-string `1` precedes the filename; only a `continue` reaches "notes.txt".
    assert analysis.filename_literals == ["notes.txt"]


def test_constant_reassigned_in_loop_body_is_demoted_as_accumulator() -> None:
    """A top-level constant that is re-bound by an annotated assignment inside a for-loop body is a
    working variable: _mutated_names must record the *real* target name (not None) so the candidate
    is demoted with the 'accumulator' reason (mutant_8 `out.add(None)`)."""
    analysis = analyze("COUNT = 0\nfor i in range(3):\n    COUNT: int = i\n")

    count = next(c for c in analysis.candidates if c.name == "COUNT")
    assert count.binding == "const"
    assert count.demoted is True
    assert count.demotion == "accumulator"


def test_shadowed_input_guard_returns_no_candidates_but_leaves_consts() -> None:
    """The `_input_candidates` shadow guard, pinned in BOTH directions. A file that binds
    `input` (here a def) yields NO input candidates (a guard removed / inverted to `if not ...`
    would surface `input-1`), while a const in the SAME file is still detected (the guard must
    not abort the whole analysis). The control below proves the guard is not firing always."""
    shadowed = "def input(p=''):\n    return 'x'\nCITY = 'Taipei'\nname = input('Name: ')\n"
    result = analyze(shadowed)
    assert [c.name for c in result.candidates if c.binding == "input"] == []
    assert [c.name for c in result.candidates if c.binding == "const"] == ["CITY"]
    # Control: without the binding, the builtin input() call IS a candidate.
    assert [
        c.name for c in analyze("name = input('Name: ')\n").candidates if c.binding == "input"
    ] == ["input-1"]
