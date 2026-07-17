"""Mutation-kill tests for skit/launcher.py — the `needs = [...]` preflight message.

_check_needs raises NotExecutableError (docker exit 126) naming every declared external command
that is not on PATH. The message is user-facing i18n copy: the exact English wording and the ", "
join between names are the observable contract a user (and the TUI/CLI that surface it) sees.
"""

from __future__ import annotations

import pytest

from skit import launcher, store

# Two names that cannot plausibly exist on PATH, so shutil.which(...) is None for both — which
# also means the join separator between them is exercised (kills the "XX, XX" join mutant).
_MISSING = ["skit-mut-missing-one", "skit-mut-missing-two"]
_EXPECTED = (
    "Missing required command(s): "
    "skit-mut-missing-one, skit-mut-missing-two"
    " — install them and retry."
)


@pytest.fixture(autouse=True)
def _isolated_dirs(tmp_path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))


def test_check_needs_message_names_missing_tools_verbatim() -> None:
    """The raised message is exactly the English template with the missing names ", "-joined —
    kills the msgid mutants (XX-wrapped / lowercased) and the "XX, XX" separator mutant."""
    entry = store.add_command("echo hi", name="needs-msg")
    entry.meta.needs = list(_MISSING)

    with pytest.raises(launcher.NotExecutableError) as exc_info:
        launcher._check_needs(entry)

    assert str(exc_info.value) == _EXPECTED


def test_preflight_surfaces_the_same_needs_message() -> None:
    """Same wording reaches the real preflight path (what the TUI calls before suspending)."""
    entry = store.add_command("echo hi", name="needs-preflight")
    entry.meta.needs = list(_MISSING)

    with pytest.raises(launcher.NotExecutableError) as exc_info:
        launcher.preflight(entry)

    assert str(exc_info.value) == _EXPECTED
