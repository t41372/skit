"""The human, translated names of the registry kinds — ONE map, shared by every
surface that shows a kind to a person (the Library badge, the kind-pick modal).

The labels must be gettext() literals (a dict lookup fed to gettext(kind) is invisible
to Babel's extractor — the i18n gate's dynamic-gettext check exists for exactly that),
and every new kind adds one literal line here.
"""

from __future__ import annotations

from .i18n import gettext


def kind_label(kind: str) -> str:
    """The translated display name for a registry kind (the raw id when unknown —
    honest for metas written by a newer skit)."""
    return {
        "python": gettext("Python"),
        "shell": gettext("Shell"),
        "fish": gettext("fish"),
        "js": gettext("JavaScript"),
        "ts": gettext("TypeScript"),
        "powershell": gettext("PowerShell"),
        "ruby": gettext("Ruby"),
        "perl": gettext("Perl"),
        "lua": gettext("Lua"),
        "r": gettext("R"),
        "exe": gettext("Program"),
        "command": gettext("Command"),
        "prompt": gettext("Prompt"),
    }.get(kind, kind)
