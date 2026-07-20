"""Mutation-kill tests for src/skit/tui.py chunk 2: ``_kind_badge``'s kind->badge map.

``_kind_badge(kind) -> (glyph, label)``. The label is looked up in a hard-coded
``{kind: gettext(DisplayName)}`` map that falls back to the raw kind for anything
unregistered; the glyph comes from the resolved language spec (a ``"?"`` placeholder when
the kind is unknown). These tests pin, against the English catalog (the conftest's
autouse ``SKIT_LANG=en``), the exact display label for every registered kind, the
case-sensitive/exact nature of the lookup, the raw-kind echo for an unknown kind, and the
``"?"`` glyph fallback.
"""

from __future__ import annotations

import pytest

from skit.tui import _kind_badge

# Every branch of the map, with the exact English display name it must resolve to.
_KIND_LABELS = [
    ("python", "Python"),
    ("shell", "Shell"),
    ("fish", "fish"),
    ("js", "JavaScript"),
    ("ts", "TypeScript"),
    ("powershell", "PowerShell"),
    ("ruby", "Ruby"),
    ("perl", "Perl"),
    ("lua", "Lua"),
    ("r", "R"),
    ("exe", "Program"),
    ("command", "Command"),
]


@pytest.mark.parametrize(("kind", "label"), _KIND_LABELS)
def test_kind_badge_label_for_registered_kind(kind: str, label: str) -> None:
    """Each registered kind resolves to its exact human display name.

    For every kind whose display name differs from the raw id (all but ``fish``), this
    also proves the map key is exactly that id: a mangled key would miss the lookup and
    the fallback would surface the raw lowercase kind instead of the display name.
    """
    assert _kind_badge(kind)[1] == label


def test_kind_badge_lookup_is_exact_and_case_sensitive() -> None:
    """The map keys are the exact lowercase kind ids.

    An unregistered spelling of ``fish`` — a differently cased (``FISH``) or mangled
    (``XXfishXX``) variant — is not silently resolved to the ``fish`` entry; it echoes
    back verbatim as the raw kind. (Pins the ``fish`` row's key, whose display name
    equals its id and so is invisible to the exact-label check above.)
    """
    assert _kind_badge("FISH")[1] == "FISH"
    assert _kind_badge("XXfishXX")[1] == "XXfishXX"


def test_kind_badge_unknown_kind_echoes_raw_kind() -> None:
    """An unknown kind falls back to the raw kind string as its label.

    The map's ``.get`` default is the kind itself — not ``None`` and not a dropped
    default — so a kind this skit version doesn't know still shows a plain badge.
    """
    assert _kind_badge("not-a-real-kind")[1] == "not-a-real-kind"


def test_kind_badge_unknown_kind_glyph_is_placeholder() -> None:
    """With no language spec resolved for an unknown kind, the glyph is the ``"?"``
    placeholder."""
    assert _kind_badge("not-a-real-kind")[0] == "?"
