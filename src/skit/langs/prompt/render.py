"""The prompt kind's two-stage render: raw substitution, no shell, no quoting.

This deliberately does NOT reuse ``TemplateLaunch._render``: that body wraps every
substituted value in ``quote_for_shell`` — correct for a ``ShellLaunch`` command string,
corrupting for prompt text (the agent would read literal quotes around every value).
Both stages are single passes over the ORIGINAL text built from match spans, so
replacement text is never re-scanned — a value that itself contains ``{{name}}``
arrives byte-identical.

There is NO escape handling on this surface (see analyzer.py): a token that isn't a
MANAGED ``{{name}}`` — any other brace shape included — is left byte-identical. The
render can only ever change the spans the user explicitly manages.
"""

from __future__ import annotations

import sys
from typing import TYPE_CHECKING

from ...i18n import gettext
from ..base import LaunchError
from .analyzer import RESERVED_NAME, TOKEN_RE

if TYPE_CHECKING:
    import re

# The whole assembled command line must stay under the platform's argv ceiling: Windows
# caps CreateProcess at 32767 UTF-16 units; Linux caps a SINGLE argv string at 128 KiB
# (MAX_ARG_STRLEN). Both are byte-ish bounds, so the check measures UTF-8 bytes with
# headroom — the refusal is skit's clean LaunchError, never a raw OS error mid-spawn.
ARGV_LIMIT = 30_000 if sys.platform == "win32" else 100_000


def render_body(text: str, values: dict[str, str], managed: list[str]) -> str:
    """Stage 1: fill the body's MANAGED ``{{placeholder}}`` tokens from ``values``, raw.

    Everything else — unmanaged ``{{name}}``, single braces, triple-stache — passes
    through byte-identical. A managed name with no value at all raises the same
    missing-values LaunchError contract as ``TemplateLaunch._render`` (assembly normally
    delivers every field's key, so this only fires on degenerate callers)."""
    missing = [name for name in managed if name not in values]
    if missing:
        raise LaunchError(
            gettext("Missing parameter values: %(names)s") % {"names": ", ".join(missing)}
        )
    managed_set = set(managed)

    def repl(m: re.Match[str]) -> str:
        name = m.group(1)
        if name not in managed_set:
            return m.group(0)
        return values[name]

    return TOKEN_RE.sub(repl, text)


def fill_runner_argv(argv: list[str], rendered: str) -> list[str]:
    """Stage 2: substitute the rendered prompt into the runner argv's one ``{{prompt}}``
    token, raw, inside its token — the result is real argv, no shell ever sees it. Any
    other brace shape in a runner token is a literal and stays byte-identical."""

    def repl(m: re.Match[str]) -> str:
        if m.group(1) == RESERVED_NAME:
            return rendered
        return m.group(0)

    return [TOKEN_RE.sub(repl, token) for token in argv]


def check_argv_length(argv: list[str]) -> None:
    """Refuse an over-long assembled command line before spawn (exit 125 via LaunchError).

    Measured in UTF-8 BYTES, not characters: the OS limits (MAX_ARG_STRLEN, the Windows
    command line) are byte/UTF-16-unit bounds, and a CJK/emoji-heavy prompt is 3-4 bytes
    per character — a character count would wave through an argv the kernel then rejects
    with a raw E2BIG at spawn."""
    total = sum(len(token.encode("utf-8")) for token in argv) + len(argv)
    if total > ARGV_LIMIT:
        raise LaunchError(
            gettext(
                "The rendered prompt makes the command line %(size)s characters — over this "
                "platform's %(limit)s limit. Shorten the prompt or its parameter values."
            )
            % {"size": total, "limit": ARGV_LIMIT}
        )
