"""Mutation-kill tests for src/skit/rewrite.py — `write_injected`'s default destination and the
temp-file naming on its entry_dir fallback path.

`write_injected` writes a possibly-secret-bearing injected script copy to a private temp file;
these pin the default-location contract (OS temp, not next to the stored script) and the exact
`.injected-<suffix>` naming used when the primary temp dir is unwritable and it falls back.
"""

from __future__ import annotations

import tempfile
from pathlib import Path

from skit import rewrite

# Captured at import, BEFORE the autouse `_sweep_injected_temp_copies` conftest fixture patches
# `rewrite.write_injected` with its `tracking` wrapper. That wrapper always forwards
# `prefer_entry_dir=prefer_entry_dir` EXPLICITLY, which masks the signature default — so to observe
# the real default we must call the underlying function directly, not the patched attribute.
_REAL_WRITE_INJECTED = rewrite.write_injected


def test_write_injected_defaults_to_os_tempdir_not_entry_dir(tmp_path: Path) -> None:
    """By default (prefer_entry_dir omitted) the injected copy lands in the OS temp dir, NOT in
    entry_dir next to the stored script — a SIGKILL before the caller's finally-unlink must not
    strand a secret-bearing file in the persistent store. Kills the signature-default mutant
    (`prefer_entry_dir: bool = False` -> `True`), under which the file would be written to
    entry_dir. Calls the real function (see `_REAL_WRITE_INJECTED`) so the default is actually
    exercised rather than being supplied explicitly by the test-suite's sweep wrapper."""
    entry_dir = tmp_path / "entry"
    entry_dir.mkdir()

    p = _REAL_WRITE_INJECTED(entry_dir, "print('hi')\n", suffix=".py")
    try:
        assert not p.is_relative_to(entry_dir)  # not written into the store dir
        assert p.parent == Path(tempfile.gettempdir())  # written to the OS temp dir
        assert p.name.startswith(".injected-")
        assert p.name.endswith(".py")
        assert p.read_text(encoding="utf-8") == "print('hi')\n"
    finally:
        p.unlink()


def test_write_injected_fallback_keeps_prefix_and_suffix(monkeypatch, tmp_path: Path) -> None:
    """When the primary temp dir (OS temp, by default) is unwritable, write_injected falls back to
    the second candidate dir (entry_dir). The fallback mkstemp must reuse the same identifiable
    `.injected-` prefix and the language-specific suffix, so the interpreter still recognizes the
    copy and sweeps can find stray injected files. Kills every prefix/suffix mutant on the fallback
    call: prefix None/dropped/XX-wrapped/case-flipped, and suffix None/dropped."""
    entry_dir = tmp_path / "entry"
    entry_dir.mkdir()

    real_mkstemp = rewrite.tempfile.mkstemp

    def _fake_mkstemp(*args, **kwargs):
        # The primary attempt targets dirs[0], which is None (OS temp) with prefer_entry_dir off.
        # Force it to fail so control reaches the entry_dir fallback (dirs[1]).
        if kwargs.get("dir") is None:
            raise OSError("simulated: OS temp dir not writable")
        return real_mkstemp(*args, **kwargs)

    monkeypatch.setattr(rewrite.tempfile, "mkstemp", _fake_mkstemp)

    p = rewrite.write_injected(entry_dir, "echo hi\n", suffix=".sh")
    try:
        assert p.parent == entry_dir  # fell back into entry_dir
        assert p.name.startswith(".injected-")  # kills prefix None/dropped/XX/.INJECTED-
        assert p.name.endswith(".sh")  # kills suffix None/dropped
        assert p.read_text(encoding="utf-8") == "echo hi\n"
    finally:
        p.unlink()
