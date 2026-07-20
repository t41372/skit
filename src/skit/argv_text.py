"""Round-trip argv through the TUI's one-line, no-shell command fields.

These fields are only an editable representation of an argv token list. They do not
invoke a shell, so parsing must preserve Windows path backslashes instead of applying
POSIX escape semantics.
"""

from __future__ import annotations

import shlex
import subprocess
import sys


def _split_windows(command: str) -> list[str]:  # noqa: PLR0912 - mirrors the CRT state machine
    """Parse the quoting convention emitted by ``subprocess.list2cmdline``.

    This is the Microsoft C runtime rule Python documents for ``list2cmdline``:
    backslashes are literal except immediately before a double quote; there, pairs
    produce literal backslashes and an odd remainder escapes the quote. Keeping this
    small inverse here avoids both POSIX shlex's path corruption and non-POSIX shlex's
    literal surrounding quotes.
    """
    argv: list[str] = []
    i = 0
    length = len(command)
    while i < length:
        while i < length and command[i] in " \t":
            i += 1
        if i == length:
            break
        token: list[str] = []
        in_quotes = False
        while i < length:
            char = command[i]
            if char in " \t" and not in_quotes:
                break
            if char == "\\":
                start = i
                while i < length and command[i] == "\\":
                    i += 1
                backslashes = i - start
                if i < length and command[i] == '"':
                    token.extend("\\" * (backslashes // 2))
                    if backslashes % 2:
                        token.append('"')
                    else:
                        in_quotes = not in_quotes
                    i += 1
                else:
                    token.extend("\\" * backslashes)
                continue
            if char == '"':
                in_quotes = not in_quotes
                i += 1
                continue
            token.append(char)
            i += 1
        if in_quotes:
            raise ValueError("No closing quotation")
        argv.append("".join(token))
    return argv


def split(command: str) -> list[str]:
    """Split one editable command line without losing native Windows paths.

    Windows uses the CRT parser paired with :func:`subprocess.list2cmdline`; POSIX uses
    shlex's native pair. No shell executes either representation.
    """
    if sys.platform == "win32":
        return _split_windows(command)
    return shlex.split(command)


def join(argv: tuple[str, ...] | list[str]) -> str:
    """Render tokens for editing using the quoting convention paired with :func:`split`."""
    if sys.platform == "win32":
        return subprocess.list2cmdline(argv)
    return shlex.join(argv)
