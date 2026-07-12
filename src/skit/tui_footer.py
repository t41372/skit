"""Shared footer convention: every bottom-bar hint is also a button.

The footer advertises the keys AND is clickable — each "<key> <label>" chip is a
Textual @click action link that fires the same action the key triggers, so mouse users
never have to learn the keys and keyboard users keep them. `action` is namespaced
("app.run" for a Library action, "screen.save_preset" for a pushed screen's own action)
so a click resolves to the right handler regardless of what currently has focus.

Rendering (btop grammar): one chip = ONE pill — a shared dark background under both the
accented key and its label, so they can't read as two separate buttons; the app theme
turns off the ansi link underline and tints the whole pill on hover.

Responsiveness: every blank inside a pill is GLUE (U+2800), so the line wrapper can
only break BETWEEN pills — on a narrow terminal a footer row wraps chip by chip, each
pill whole and clickable on its own line. Key rows live in a KeysBar: the height tiers
(tui_layout) cap how many wrapped lines are VISIBLE, and the bar scrolls, so a capped
tier trims what is on screen — never what the mouse can reach. Chips are ordered most
important first, so the visible lines are always the most useful ones.
"""

from __future__ import annotations

import re

from textual.binding import Binding
from textual.containers import Vertical, VerticalScroll

from .i18n import gettext

# The gap between pills. The pill backgrounds already separate the buttons, so the
# old middle-dot separator would just be noise between them.
SEP = "  "

# The pill: a warm near-invisible lift off the terminal background — enough to bind
# key+label into one button shape without turning the footer into a lightbar.
_PILL_BG = "#2A211C"

# The blank inside a pill: U+2800 BRAILLE PATTERN BLANK. It renders as a space (and
# keeps the pill background), but it is not str-whitespace, so Textual's wrapper —
# which chunks lines on \s — sees each pill as one unbreakable word and wraps only
# at the plain spaces of SEP. A real no-break space (U+00A0) does NOT work: \s
# matches it and the pill would snap apart mid-label.
GLUE = "⠀"

# Textual's wrapper breaks on regex \s, so EVERY \s blank in a pill must become GLUE —
# not just ASCII space: a translated label with an NBSP (French "Aide ?") or an
# ideographic space (U+3000) would otherwise snap its pill apart mid-label.
_BLANK = re.compile(r"\s")


def chip(action: str, key: str, label: str) -> str:
    """One clickable footer pill. The key glyph is accented; the whole pill is the link.

    key/label are trusted UI literals (never user input), so they carry no markup to
    escape; action is a fixed action string. The single tag carries the click action
    AND the pill background so every cell of the chip belongs to the same button; the
    inner span only recolors the key. An empty label yields a key-only pill (used by the
    field-nav hint, where the ↓/↑ arrows already carry the meaning and footer space is
    tight). Every blank inside the pill is GLUE so the pill wraps as one unit.
    """
    key = _BLANK.sub(GLUE, key)
    label = _BLANK.sub(GLUE, label)
    inner = f"[bold $accent]{key}[/]{GLUE}{label}" if label else f"[bold $accent]{key}[/]"
    return f"[on {_PILL_BG} @click={action}]{GLUE}{inner}{GLUE}[/]"


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
    """The shared "how do I move?" hint for form footers — BOTH directions. A footer that
    advertises only forward silently strands anyone who tabs one field too far: the way back
    is real but was invisible. So this is two pills, forward (Tab/↓) and back (Shift+Tab/↑),
    each clickable — click fires focus_next / focus_previous, the same actions the keys
    trigger, so the mouse always has the path the keyboard does. They're key-only (no "Next
    field" / "Previous field" text): the ↓/↑ arrows already say which way, and the crowded
    add / run-form footers have no room for two full labels. The words still live on the
    FIELD_NAV_BINDINGS below, so the help screen and any binding list keep the full names."""
    return bar(
        chip("app.focus_next", "Tab/↓", ""),
        chip("app.focus_previous", "Shift+Tab/↑", ""),
    )


class KeysBar(Vertical):
    """The footer key-row area every skit screen shares. Rows wrap chip-by-chip (the
    pills are unbreakable — GLUE) and the height tiers cap how many wrapped lines are
    VISIBLE; the bar itself scrolls, so a capped tier trims visibility, never the
    mouse path — every chip stays wheel-reachable on any terminal, which is what lets
    the caps coexist with the mouse-alone policy (AGENTS.md principle 2). Keyboard
    users never need to scroll it: every chip's key keeps firing regardless.

    One widget owns the tier caps so screens can't drift apart; a screen that needs a
    different budget (the Library shows two rows) overrides by id. SCOPED_CSS off:
    the tier rules must see the -h-* classes on the ancestor Screen.
    """

    SCOPED_CSS = False
    DEFAULT_CSS = """
    KeysBar { height: auto; overflow-y: auto; }
    KeysBar > Static { height: auto; padding: 0 1; }
    Screen.-h-normal KeysBar { max-height: 3; }
    Screen.-h-short KeysBar, Screen.-h-tiny KeysBar { max-height: 1; }
    """


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
