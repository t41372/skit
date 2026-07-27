"""The human, translated VALUE LABELS for skit's stored enum tokens — kind, mode and
workdir. ONE map per axis, shared by every surface that shows one to a person.

The token stays English wherever it is stored or typed (`meta.toml`, `skit params
--workdir store`); only what a person READS is translated. Without that split, `skit
show` printed `(Python · copy)` — the kind translated and the mode raw, inside one
parenthesis — and `工作目錄:store`. No static gate could ever catch it: the literal is not
in the source at all, it is in the user's meta.toml.

Scope, stated so it does not drift: this module owns compact VALUE labels. The prose a
screen writes to explain a CHOICE ("The source file's folder", offered beside a radio
button) belongs to that screen — it is a different register, and forcing the two together
would make both worse.

Every label must be a gettext() literal (a dict lookup fed to gettext(token) is invisible
to Babel's extractor — the i18n gate's dynamic-gettext check exists for exactly that), so
every new kind, mode or workdir adds one literal line here.
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


def mode_label(mode: str) -> str:
    """The translated display name for a storage mode (the raw token when unknown)."""
    return {
        "copy": gettext("copy"),
        "reference": gettext("reference"),
    }.get(mode, mode)


def workdir_label(workdir: str) -> str:
    """The translated display name for a workdir setting.

    Falls through unchanged for anything that is not one of the three tokens, because
    `workdir` also holds a user-typed ABSOLUTE PATH — translating that would corrupt it.
    The `.get(token, token)` idiom above carries the same rule for a kind from a newer
    skit."""
    return {
        "origin": gettext("the script's own folder"),
        "store": gettext("skit's stored-copy folder"),
        "invoke": gettext("wherever skit is run from"),
    }.get(workdir, workdir)


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
