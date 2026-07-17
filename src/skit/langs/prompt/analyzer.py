"""Placeholder detection for prompt bodies: double-brace tokens, nothing else.

A prompt's holes are spelled ``{{name}}`` (identifier body) — the convention of the
prompt-template world (Anthropic Console variables, Jinja2, Handlebars, Mustache),
chosen over the command-template ``{name}`` precisely because prompts quote code:
JSON, shell ``${VAR}``, f-strings and friends are full of single-brace identifiers
that are NOT parameters. Guards exclude a brace-adjacent match, so a Handlebars
triple-stache ``{{{raw}}}`` is not a candidate either.

There is deliberately NO escape sequence: an unmanaged ``{{name}}`` (a Jinja example,
say) passes through to the agent byte-identical, so nothing in a body ever needs
escaping — what the user hasn't managed, skit doesn't touch. Detection therefore only
proposes CANDIDATES; the managed list (`meta.params`) is what the form asks for and
the renderer fills.
"""

from __future__ import annotations

import re

# The one insertion-marker grammar for the prompt surface (bodies AND runner argv
# slots): {{identifier}}, not brace-adjacent. Command templates keep their own {name}
# grammar in langs/launch.py — that surface shipped first and is shell-quoted, a
# different world.
TOKEN_RE = re.compile(r"(?<!\{)\{\{([a-zA-Z_][a-zA-Z0-9_]*)\}\}(?!\})")

# `prompt` is reserved: it names the runner template's own slot, and a form field called
# "prompt" on a prompt entry would be endless confusion (an ergonomic guard, not a
# mechanical collision — body holes are stage 1, the runner slot is stage 2). It is never
# offered as a candidate and never managed; a literal {{prompt}} in a body passes through
# to the agent verbatim.
RESERVED_NAME = "prompt"

# Flood guards for prompts that were never written with insertion in mind (a long prompt
# can trip hundreds of candidates). Above AUTO_MANAGE_LIMIT detections, nothing is
# auto-managed — the entry stays runnable verbatim and the user opts holes in one by one.
# LIST_PREVIEW_LIMIT caps how many candidate NAMES any list surface prints/renders.
AUTO_MANAGE_LIMIT = 30
LIST_PREVIEW_LIMIT = 20


def placeholder_names(text: str) -> list[str]:
    """Every manageable ``{{name}}`` in a prompt body, deduped in order of first
    appearance. The reserved name is excluded outright (see RESERVED_NAME)."""
    seen: list[str] = []
    for m in TOKEN_RE.finditer(text):
        name = m.group(1)
        if name != RESERVED_NAME and name not in seen:
            seen.append(name)
    return seen


def preview_names(names: list[str]) -> str:
    """A flood-safe, comma-joined rendering of candidate names: the first
    LIST_PREVIEW_LIMIT, then a "+N more" tail. Pure formatting (no i18n — names are
    data; callers wrap the sentence around it)."""
    if len(names) <= LIST_PREVIEW_LIMIT:
        return ", ".join(names)
    shown = ", ".join(names[:LIST_PREVIEW_LIMIT])
    return f"{shown} … +{len(names) - LIST_PREVIEW_LIMIT}"
