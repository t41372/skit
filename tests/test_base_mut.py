"""Mutation-kill tests for skit/langs/base.py injection-error messages.

InjectGapError and InjectSplitError are raised by the shell injector when a value can't be
delivered safely through a `read`. Each carries structured fields *and* a str() form the caller
falls back to for its message, so the exact str() text is observable behaviour (mutants replace it
with super().__init__(None), collapsing the message to "None").
"""

from __future__ import annotations

from skit.langs.base import InjectGapError, InjectSplitError


def test_inject_gap_error_message_and_fields() -> None:
    exc = InjectGapError("FIRST", "LAST")
    assert exc.empty == "FIRST"
    assert exc.filled == "LAST"
    # str() form is "{empty} < {filled}" — kills super().__init__(None) (-> "None").
    assert str(exc) == "FIRST < LAST"


def test_inject_split_error_message_and_fields() -> None:
    exc = InjectSplitError("NAME", "line-break")
    assert exc.name == "NAME"
    assert exc.reason == "line-break"
    # str() form is "{name}: {reason}" — kills super().__init__(None) (-> "None").
    assert str(exc) == "NAME: line-break"
