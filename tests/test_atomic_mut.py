"""Mutation-kill tests for skit/atomic.py — the final os.replace in _replace_with_retry.

After the bounded Windows sharing-violation backoff is exhausted, _replace_with_retry makes one
last os.replace(src, dst). That final attempt must still pass the *real* src and dst (not None):
it is what actually lands the write when the transient contention clears on the last try. The
mutants pass None for one operand; both make that final replace raise instead of moving the file.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from skit import atomic


def test_final_replace_after_exhausted_retries_moves_the_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Seven sharing violations, then success on the eighth (final, post-loop) attempt: the file
    must actually move, which requires the final os.replace to carry the real (src, dst). A None
    operand (mutant_11 `os.replace(None, dst)` / mutant_12 `os.replace(src, None)`) raises there
    instead, so the write never lands."""
    real_replace = os.replace
    StrPath = str | os.PathLike[str]
    calls: list[tuple[StrPath, StrPath]] = []

    def flaky_replace(src: StrPath, dst: StrPath) -> None:
        calls.append((src, dst))
        if len(calls) <= 7:  # every retry inside the loop fails transiently
            raise PermissionError(13, "sharing violation")
        real_replace(src, dst)  # the final loud attempt performs the real rename

    monkeypatch.setattr(atomic.os, "replace", flaky_replace)
    monkeypatch.setattr(atomic.time, "sleep", lambda _s: None)  # don't actually back off ~1.3s

    src = tmp_path / "scratch.tmp"
    src.write_bytes(b"payload")
    dst = tmp_path / "registry.toml"

    atomic._replace_with_retry(str(src), dst)

    assert dst.read_bytes() == b"payload"  # the write landed on the final attempt
    assert not src.exists()  # a real rename consumed the source
    assert len(calls) == 8  # 7 retried in the loop + 1 final attempt reached
    assert calls[-1] == (str(src), dst)  # the final attempt used the real operands, not None
