"""Mutation-kill tests for skit/langs/python/analyzer.py.

Two detection details with real add-time consequences:
- filename-literal hints must keep scanning a call's later arguments past a non-string one
  (`continue`, not `break`), or a filename after a non-string positional is missed.
- a constant reassigned inside a loop body (AnnAssign target) is a working variable, not a
  parameter, so the candidate is demoted to unchecked at onboarding.
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
