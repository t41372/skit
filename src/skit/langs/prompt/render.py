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

import os
import subprocess
import sys
from typing import TYPE_CHECKING

from ...i18n import gettext
from ..base import LaunchError
from .analyzer import RESERVED_NAME, TOKEN_RE

if TYPE_CHECKING:
    import re

# The whole assembled command line must stay under the platform's argv ceiling. Windows
# caps CreateProcess at 32767 UTF-16 code units INCLUDING its terminator; Python first
# quotes argv with list2cmdline, which can double backslashes before quotes. Measure that
# exact UTF-16 byte string with 5534 bytes of headroom. Linux caps a SINGLE argv string
# at 128 KiB (MAX_ARG_STRLEN); the conservative POSIX total stays at 100 KiB.
ARGV_LIMIT = 60_000 if sys.platform == "win32" else 100_000

# Pi handles these exact first arguments before its normal message parser, while
# dash-prefixed and @-prefixed arguments are parsed as options and file attachments.
# Pi currently has no end-of-options delimiter, so its argv compatibility adapter
# prefixes one newline only for those ambiguous prompts.
PI_PACKAGE_COMMANDS = frozenset({"config", "install", "list", "remove", "uninstall", "update"})


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
        if not name.isidentifier() or name not in managed_set:
            return m.group(0)
        return values[name]

    return TOKEN_RE.sub(repl, text)


def protect_pi_prompt(rendered: str) -> tuple[str, bool]:
    """Make a Pi opening prompt unambiguously positional when its parser would not.

    The fallback is deliberately non-lossless and therefore returns whether it was
    applied: delivery callers must surface the accompanying warning before launch.
    A single leading newline is the smallest transformation that defeats all of Pi's
    current first-argument dispatch checks while preserving interactive mode.
    """
    ambiguous = rendered.startswith(("-", "@")) or rendered in PI_PACKAGE_COMMANDS
    return (f"\n{rendered}", True) if ambiguous else (rendered, False)


def fill_runner_argv(argv: list[str], rendered: str, extra: list[str] | None = None) -> list[str]:
    """Stage 2: substitute the rendered prompt into the runner argv's one ``{{prompt}}``
    token, raw, inside its token — the result is real argv, no shell ever sees it. Any
    other brace shape in a runner token is a literal and stays byte-identical."""

    def repl(m: re.Match[str]) -> str:
        if m.group(1) == RESERVED_NAME:
            return rendered
        return m.group(0)

    filled = [TOKEN_RE.sub(repl, token) for token in argv]
    if not extra:
        return filled
    delimiter = next((i for i, piece in enumerate(argv) if piece == "--"), None)
    if delimiter is None:
        return [*filled, *extra]
    # A positional prompt needs `--` to prevent its first word being parsed as an
    # agent option. Per-run flags still belong to the option side of that boundary.
    return [*filled[:delimiter], *extra, *filled[delimiter:]]


def check_argv_length(argv: list[str]) -> None:
    """Refuse an over-long assembled command line before spawn (exit 125 via LaunchError).

    Measured in the bytes the platform will actually receive: UTF-8 argv bytes on
    POSIX; on Windows, Python's quoted command line encoded as UTF-16LE plus its NUL
    terminator. A raw character/argv count can miss both non-ASCII expansion and
    list2cmdline's backslash doubling."""
    if any("\x00" in token for token in argv):
        raise LaunchError(
            gettext(
                "The rendered prompt contains a NUL byte, which can't be passed in a process argument."
            )
        )
    try:
        total = (
            len(subprocess.list2cmdline(argv).encode("utf-16-le")) + 2
            if sys.platform == "win32"
            else sum(len(os.fsencode(token)) for token in argv) + len(argv)
        )
    except UnicodeEncodeError as exc:
        raise LaunchError(
            gettext(
                "The rendered prompt contains text this platform can't encode as a process argument."
            )
        ) from exc
    if total > ARGV_LIMIT:
        raise LaunchError(
            gettext(
                "The rendered prompt makes the command line %(size)s bytes — over this "
                "platform's %(limit)s-byte limit. Shorten the prompt or its parameter values."
            )
            % {"size": total, "limit": ARGV_LIMIT}
        )
