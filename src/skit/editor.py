"""Launch the user's editor to write or edit a script's source.

Resolution precedence for which editor to run: config.toml `editor` > $VISUAL > $EDITOR > a platform
default (`notepad` on Windows, `vi` elsewhere). The configured value may carry arguments
(e.g. `code --wait`), split with shlex; the file path is appended as the final argument.

Headless: imports neither CLI nor TUI, so store/launcher paths can use it too.
"""

from __future__ import annotations

import os
import shlex
import subprocess
import sys
from pathlib import Path

from . import config
from .i18n import gettext


class EditorError(Exception):
    """The editor could not be launched (e.g. the command was not found on PATH)."""


class EditedSourceError(Exception):
    """The editor returned, but the resulting entry source cannot be accepted."""


def _platform_default() -> str:
    return "notepad" if sys.platform == "win32" else "vi"


def resolve_editor() -> list[str]:
    """The editor command as an argv prefix (the file path is appended by open_in_editor).

    Falls back to the platform default when nothing is configured and neither env var is set. A
    candidate that is blank or whitespace-only (e.g. VISUAL="  ") is treated as unset so the next
    candidate in the precedence chain — not the platform default — gets a chance.
    """
    visual = os.environ.get("VISUAL", "")  # pragma: no mutate — default "" vs None are both falsy
    editor_env = os.environ.get(
        "EDITOR", ""
    )  # pragma: no mutate — default "" vs None are both falsy
    candidates = (config.load_editor(), visual, editor_env)
    raw = next((c.strip() for c in candidates if c.strip()), _platform_default())
    try:
        parts = shlex.split(raw, posix=sys.platform != "win32")
    except ValueError:
        # An unbalanced-quote value is unusable as a parsed command; treat the whole thing as the
        # program name rather than crashing.
        parts = [raw]
    if sys.platform == "win32":
        # Non-posix shlex preserves backslashes (so C:\tools\edit.exe survives intact) but it also
        # keeps a token's surrounding double-quotes literally. A quoted spaced path (the normal way
        # to write one on Windows, e.g. "C:\Program Files\...\Code.exe") would otherwise reach
        # CreateProcess with the quote characters baked into the filename, which it can never find.
        # Strip one matching pair of surrounding quotes per token to fix that.
        parts = [p[1:-1] if len(p) >= 2 and p[0] == p[-1] == '"' else p for p in parts]
    return parts or [_platform_default()]


def open_in_editor(path: Path) -> int:
    """Open `path` in the resolved editor and block until it exits; return the editor's exit code.

    Raises EditorError only when the editor cannot be launched at all (a non-zero exit is returned,
    not raised — some editors exit non-zero on an unmodified close).
    """
    argv = [*resolve_editor(), str(path)]
    try:
        # check=False is subprocess.run's default; keeping it explicit. noqa: S603 — argv from the
        # user-configured editor.
        completed = subprocess.run(argv, check=False)  # noqa: S603  # pragma: no mutate
    except OSError as exc:
        raise EditorError(
            gettext(
                "Could not launch the editor (%(cmd)s): %(error)s. Set one with: skit config editor <cmd>"
            )
            % {"cmd": " ".join(argv[:-1]), "error": str(exc)}
        ) from exc
    return completed.returncode


def open_entry_in_editor(path: Path, *, kind: str) -> int:
    """Edit an existing entry source, then validate kind-specific payload invariants.

    Prompt bodies are an exact UTF-8 argv payload, so replacement-character decoding
    is never an acceptable edit result.  The bytes the editor wrote stay at ``path``
    on refusal: that preserves the user's work (and, in reference mode, never rewrites
    their original behind their back), while the same edit action remains the recovery
    path.  New-entry draft flows deliberately keep using :func:`open_in_editor`; their
    review/onboarding stages already own stricter keep-the-draft behavior.
    """
    returncode = open_in_editor(path)
    if kind != "prompt":
        return returncode
    from .langs.prompt import text as prompt_text

    try:
        prompt_text.read(path)
    except prompt_text.PromptEncodingError as exc:
        raise EditedSourceError(str(exc)) from exc
    except OSError as exc:
        raise EditedSourceError(
            gettext("Can't read %(path)s: %(error)s")
            % {"path": str(path), "error": exc.strerror or str(exc)}
        ) from exc
    return returncode
