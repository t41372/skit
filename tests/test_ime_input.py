"""CJK IME regression guard (the "can't type Chinese in iTerm2" incident).

Textual ≥ 8.2.7 pushes the kitty keyboard protocol with "report all keys as escape
codes" + "report associated text" (`ESC [>25u`). iTerm2 3.6.x implements that mode in
a way that fights the macOS IME: the digit that picks a candidate and the Enter that
commits it arrive as raw key events (inserting a literal "1" / firing the Enter
binding) while the composed CJK text is never delivered; Caps Lock even lands as a
literal "A". No skit binding needs the protocol's extras, so src/skit/__init__.py
opts out via TEXTUAL_DISABLE_KITTY_KEY. The opt-out only counts if it lands before
textual.constants is imported — textual reads the env var once, at import time.
"""

from __future__ import annotations

import importlib
import os

import skit


def test_kitty_protocol_opt_out_is_set_at_package_import(monkeypatch):
    monkeypatch.delenv("TEXTUAL_DISABLE_KITTY_KEY", raising=False)
    importlib.reload(skit)
    assert os.environ["TEXTUAL_DISABLE_KITTY_KEY"] == "1"


def test_kitty_protocol_opt_out_respects_an_explicit_user_override(monkeypatch):
    """setdefault, not assignment: =0 must survive so a user can re-enable the protocol
    (e.g. on a terminal whose kitty implementation coexists with their IME)."""
    monkeypatch.setenv("TEXTUAL_DISABLE_KITTY_KEY", "0")
    importlib.reload(skit)
    assert os.environ["TEXTUAL_DISABLE_KITTY_KEY"] == "0"


def test_kitty_protocol_opt_out_lands_before_textual_reads_it():
    """End-to-end wiring: in any process that imports skit first (the console script
    guarantees it — `skit.cli:app` runs the package __init__ before anything else),
    textual must actually see the opt-out. Guards against the flag moving somewhere
    that loads after textual.constants."""
    from textual import constants

    assert constants.DISABLE_KITTY_KEY is True
