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

from textual.binding import Binding
from textual.containers import VerticalScroll

from .i18n import gettext

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


# Every form-style screen shares the same movement keys: Tab is Textual's native focus
# order, and ↓/↑ are its arrow twins for anyone who reaches for arrows first. A widget
# that needs the arrows for itself (RadioSet options, an open Select) wins — these fire
# only when the focused widget lets the key through.
FIELD_NAV_BINDINGS = (
    Binding("down", "app.focus_next", gettext("Next field"), show=False),
    Binding("up", "app.focus_previous", gettext("Previous field"), show=False),
)


def nav_chip() -> str:
    """The shared "how do I move?" hint for form footers. Clicking it steps to the
    next field, same as the keys it advertises."""
    return chip("app.focus_next", "Tab/↓", gettext("Next field"))


class FormBody(VerticalScroll):
    """The scrolling body of a form screen. Its arrows move FOCUS, not the scrollbar:
    once a form overflows, VerticalScroll would otherwise swallow ↓/↑ for scrolling
    and the advertised field navigation silently dies exactly when the form is big.
    Focus changes auto-scroll their target into view; PageUp/PageDown and the wheel
    still scroll."""

    BINDINGS = [
        Binding("down", "app.focus_next", gettext("Next field"), show=False),
        Binding("up", "app.focus_previous", gettext("Previous field"), show=False),
    ]
