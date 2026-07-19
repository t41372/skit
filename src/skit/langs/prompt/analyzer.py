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

# The one double-brace token SHAPE for prompt bodies and runner argv. Whether the
# contents are a manageable name is decided with str.isidentifier(), which gives a
# localized prompt the same natural identifier vocabulary Python does (``{{任务}}``,
# ``{{café}}``, …). Matching the broader shape is deliberate: runner validation must
# reject an unsupported ``{{not-a-name}}`` instead of overlooking it as literal data.
# Triple-stache remains excluded by the brace-adjacency guards.
TOKEN_RE = re.compile(r"(?<!\{)\{\{([^{}]*)\}\}(?!\})")

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
    names: list[str] = []
    seen: set[str] = set()
    for m in TOKEN_RE.finditer(text):
        name = m.group(1)
        if name != RESERVED_NAME and name.isidentifier() and name not in seen:
            seen.add(name)
            names.append(name)
    return names


def preview_names(names: list[str]) -> tuple[str, int]:
    """Flood-safe preview data: comma-joined visible names and the hidden count.

    Names are data, but a truncation tail is user-interface grammar (``and N more``)
    and belongs at the caller's i18n boundary.  Keeping the count separate lets a
    human surface translate the whole sentence while a machine surface keeps returning
    the complete list unchanged.
    """
    shown = names[:LIST_PREVIEW_LIMIT]
    return ", ".join(shown), len(names) - len(shown)
