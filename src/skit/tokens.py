"""Value tokens: run-time placeholders usable inside any form value.

`{cwd}` (invoke-time working directory), `{today}` (YYYY-MM-DD), `{now}` (HH-MM-SS),
`{env:NAME}` (environment variable), and a leading `~` (home). Values store the token
TEXT (intent), never the expanded result — argstate/presets persist `out_{today}.png`,
and every run expands fresh (the same rule that keeps `shots/*.png` a living glob).

Expansion contract:
- Only the known token names above are expanded. Any other `{...}` passes through
  untouched — a value may legitimately contain braces destined for the script itself.
- `{{` and `}}` escape to literal `{` / `}` (the way to pass a literal `{cwd}` through).
- A missing environment variable is an error, never a silent empty string — a command
  quietly assembled around "" is exactly the kind of breakage the non-interactive
  contract forbids.

This module is headless and stdlib-only; `cwd`/`env`/`now` are injectable for tests.
"""

from __future__ import annotations

import os
import re
from collections.abc import Mapping
from datetime import datetime
from pathlib import Path

from .i18n import gettext

# {cwd} / {today} / {now} / {env:NAME}; NAME follows the usual environment-variable shape.
_TOKEN_RE = re.compile(r"\{(cwd|today|now|env:(?P<env>[A-Za-z_][A-Za-z0-9_]*))\}")


class TokenError(ValueError):
    """A token cannot be expanded (currently: the named environment variable is unset)."""


def expand(
    text: str,
    *,
    cwd: Path | str,
    env: Mapping[str, str] | None = None,
    now: datetime | None = None,
) -> str:
    """Expand tokens in text and return the final value. Raises TokenError."""
    if env is None:
        env = os.environ
    if now is None:
        now = datetime.now()
    if text.startswith("~"):
        text = os.path.expanduser(text)
    out: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        two = text[i : i + 2]
        if two == "{{":
            out.append("{")
            i += 2
            continue
        if two == "}}":
            out.append("}")
            i += 2
            continue
        if ch == "{":
            m = _TOKEN_RE.match(text, i)
            if m is not None:
                out.append(_resolve(m, cwd=cwd, env=env, now=now))
                i = m.end()
                continue
        out.append(ch)
        i += 1
    return "".join(out)


def _resolve(m: re.Match[str], *, cwd: Path | str, env: Mapping[str, str], now: datetime) -> str:
    name = m.group(1)
    if name == "cwd":
        return str(cwd)
    if name == "today":
        return now.strftime("%Y-%m-%d")
    if name == "now":
        return now.strftime("%H-%M-%S")
    env_name = m.group("env")
    if env_name not in env:
        raise TokenError(
            gettext("The environment variable %(name)s isn't set (needed by %(token)s).")
            % {"name": env_name, "token": m.group(0)}
        )
    return env[env_name]


def preview(
    text: str,
    *,
    cwd: Path | str,
    env: Mapping[str, str] | None = None,
    now: datetime | None = None,
) -> tuple[str, str | None]:
    """Non-raising expand for live form previews: (expanded, None) on success,
    (original text, error message) when a token can't be expanded."""
    try:
        return expand(text, cwd=cwd, env=env, now=now), None
    except TokenError as exc:
        return text, str(exc)


def has_tokens(text: str) -> bool:
    """Whether expand() would change text (used to decide if a preview line is worth showing)."""
    return (
        text.startswith("~") or "{{" in text or "}}" in text or _TOKEN_RE.search(text) is not None
    )
