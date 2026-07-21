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


def kind_choices(*, offer_exe: bool) -> list[tuple[str, str]]:
    """The (kind, label) options of the unclassifiable-file ASK, in display order —
    ONE list rendered by both faces (KindPickModal's options and the plain form's
    numbered menu), so the twins cannot drift. "prompt" is family "interpreted" too,
    but it gets its OWN dedicated wording at the end (and, in the modal, listing it
    twice would duplicate the option id); exe is gated because the draft lanes
    withhold it (authored text is never a binary, and the drafts boundary refuses
    exe entries outright)."""
    from .langs.registry import KNOWN_KINDS, spec_for

    interpreted = sorted(
        k
        for k in KNOWN_KINDS
        if (spec := spec_for(k)) is not None and spec.family == "interpreted" and k != "prompt"
    )
    choices = [(k, kind_label(k)) for k in interpreted]
    if offer_exe:
        choices.append(("exe", gettext("A program (run it directly)")))
    choices.append(("prompt", gettext("A prompt for an AI agent")))
    return choices
