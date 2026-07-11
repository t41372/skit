"""Shared footer convention: every bottom-bar hint is also a button.

The footer advertises the keys AND is clickable — each "<key> <label>" chip is a
Textual @click action link that fires the same action the key triggers, so mouse users
never have to learn the keys and keyboard users keep them. `action` is namespaced
("app.run" for a Library action, "screen.save_preset" for a pushed screen's own action)
so a click resolves to the right handler regardless of what currently has focus.

Rendering (btop grammar): one chip = ONE pill — a shared dark background under both the
accented key and its label, so they can't read as two separate buttons; the app theme
turns off the ansi link underline and tints the whole pill on hover.
"""

from __future__ import annotations

# The gap between pills. The pill backgrounds already separate the buttons, so the
# old middle-dot separator would just be noise between them.
SEP = "  "

# The pill: a warm near-invisible lift off the terminal background — enough to bind
# key+label into one button shape without turning the footer into a lightbar.
_PILL_BG = "#2A211C"


def chip(action: str, key: str, label: str) -> str:
    """One clickable footer pill. The key glyph is accented; the whole pill is the link.

    key/label are trusted UI literals (never user input), so they carry no markup to
    escape; action is a fixed action string. The single tag carries the click action
    AND the pill background so every cell of the chip belongs to the same button; the
    inner span only recolors the key.
    """
    return f"[on {_PILL_BG} @click={action}] [bold $accent]{key}[/] {label} [/]"


def bar(*chips: str) -> str:
    """Join chips into a footer line."""
    return SEP.join(chips)
