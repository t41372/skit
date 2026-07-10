"""Regression test for a real incident: mutation testing escaping test isolation.

`uv run mutmut run` mutates src/skit/paths.py itself, including the "SKIT_DATA_DIR"
(etc.) string literals that tests/conftest.py's autouse `_isolate_skit_dirs` fixture
relies on to redirect skit's directories into tmp_path. When such a mutant corrupts
the env-var key, the SKIT_* lookup in paths.py silently misses and falls through to
its platformdirs-based default — which resolves against the developer's REAL
~/Library/Application Support/skit (macOS) or ~/.local/share/skit (Linux). This
actually happened: it clobbered a real registry entry with ghost data referencing
pytest tmp paths.

The fix is a second isolation layer: the same fixture also redirects HOME (and the
XDG_*_HOME vars on Linux) so that even if the SKIT_* lookup is mutated away entirely,
platformdirs' fallback still resolves inside tmp_path rather than the real user
directories. This test simulates exactly that failure mode directly against
skit.paths, without going through mutmut, so the protection is verified on every
normal test run.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import paths


def test_platformdirs_fallback_stays_isolated_when_skit_env_missing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Simulate a mutant that breaks the SKIT_DATA_DIR/STATE/CONFIG lookup.

    Even with those env vars entirely absent (as if paths.py's os.environ.get key
    were corrupted by a mutant), data_dir()/state_dir()/config_dir() must still
    resolve inside the fixture's fake HOME (set by conftest._isolate_skit_dirs) —
    never inside the developer's real home directory.
    """
    monkeypatch.delenv("SKIT_DATA_DIR", raising=False)
    monkeypatch.delenv("SKIT_STATE_DIR", raising=False)
    monkeypatch.delenv("SKIT_CONFIG_DIR", raising=False)

    # confirms conftest's HOME redirect is in effect (the platformdirs fallback below
    # may resolve via HOME, via XDG_*_HOME, or both, depending on platform — either
    # way it must land under tmp_path, never under the developer's real home).
    assert Path.home() == tmp_path / "home"

    for resolver in (paths.data_dir, paths.state_dir, paths.config_dir):
        resolved = resolver()
        assert tmp_path in resolved.parents
