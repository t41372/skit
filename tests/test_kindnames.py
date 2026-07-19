"""The one translated kind-name map (src/skit/kindnames.py), shared by the Library badge
and the KindPickModal. Every registered kind must have a literal label (the i18n gate can
only extract literals), and an unknown kind must fall through to its raw id."""

from __future__ import annotations

import pytest

from skit import kindnames
from skit.langs.registry import KNOWN_KINDS

# The English label each registered kind renders as (msgids ARE the English source).
EXPECTED = {
    "python": "Python",
    "shell": "Shell",
    "fish": "fish",
    "js": "JavaScript",
    "ts": "TypeScript",
    "powershell": "PowerShell",
    "ruby": "Ruby",
    "perl": "Perl",
    "lua": "Lua",
    "r": "R",
    "exe": "Program",
    "command": "Command",
    "prompt": "Prompt",
}


@pytest.fixture(autouse=True)
def _english(monkeypatch):
    monkeypatch.setenv("SKIT_LANG", "en")


@pytest.mark.parametrize(("kind", "label"), sorted(EXPECTED.items()))
def test_kind_label_maps_each_registered_kind(kind, label):
    assert kindnames.kind_label(kind) == label


def test_every_known_kind_has_a_dedicated_label():
    """No registered kind may fall through to the raw-id branch — a kind rendering as its
    bare id in the Library badge is an untranslated leak (the map is the i18n contract).
    'fish' is the one kind whose label is intentionally its own id."""
    for kind in KNOWN_KINDS:
        assert kind in EXPECTED, f"registered kind missing an expected label: {kind}"
        rendered = kindnames.kind_label(kind)
        # A mapped kind never returns via the `.get(kind, kind)` fallthrough — its label is
        # the literal above (which, for 'fish', happens to equal the id — still a real hit).
        assert rendered == EXPECTED[kind]


def test_unknown_kind_falls_through_to_its_raw_id():
    """A meta written by a newer skit (an unknown kind) renders honestly as its raw id,
    never a crash or a blank — the `.get(kind, kind)` fallthrough."""
    assert kindnames.kind_label("cobol") == "cobol"
    assert kindnames.kind_label("") == ""
