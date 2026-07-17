"""Placeholder detection for prompt bodies: the same token grammar command templates use.

A prompt's `{name}` holes are found with TemplateLaunch's own `_TEMPLATE_TOKEN_RE`
(imported, not copied — the two surfaces must never drift on what counts as a
placeholder): `{name}` with an identifier body, `{{`/`}}` as literal-brace escapes.
Prompts routinely contain code snippets, so a detected name is only a *candidate* —
the managed list (`meta.params`) is what the form asks for and the renderer fills.
"""

from __future__ import annotations

from ..launch import _TEMPLATE_TOKEN_RE as TOKEN_RE

# `prompt` is reserved: it names the runner template's own slot, and a form field called
# "prompt" on a prompt entry would be endless confusion (an ergonomic guard, not a
# mechanical collision — body holes are stage 1, the runner slot is stage 2). It is never
# offered as a candidate and never managed; a literal {prompt} in a body passes through
# to the agent verbatim.
RESERVED_NAME = "prompt"


def placeholder_names(text: str) -> list[str]:
    """Every manageable `{name}` in a prompt body, deduped in order of first appearance.
    The reserved name is excluded outright (see RESERVED_NAME)."""
    seen: list[str] = []
    for m in TOKEN_RE.finditer(text):
        name = m.group(1)
        if name is not None and name != RESERVED_NAME and name not in seen:
            seen.append(name)
    return seen
