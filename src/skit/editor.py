"""Launch the user's editor to write or edit a script's source.

Resolution precedence for which editor to run: config.toml `editor` > $VISUAL > $EDITOR > a platform
default (`notepad` on Windows, `vi` elsewhere). The configured value may carry arguments
(e.g. `code --wait`), split with shlex; the file path is appended as the final argument.

Headless: imports neither CLI nor TUI, so store/launcher paths can use it too.
"""

from __future__ import annotations

import contextlib
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from . import config, interaction
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
    if not interaction.allowed():
        # THE gate, at the one door every editor lane passes through. Two of the four
        # lanes refused on their own (`add --edit`, `add --prompt`) and two did not —
        # including `skit edit`, which the bundled Agent Skill teaches. In a pipe or under
        # --no-input that spawned $EDITOR against a stdin nobody is typing into: `vi` hung
        # forever, `cat` dumped the file into the caller's stdout, and skit then printed
        # "Saved" about an edit that could not have happened. An editor session IS
        # interaction (the words the refusing lanes already use), so the rule belongs here
        # rather than in each caller that has to remember it — round 10's lesson, applied
        # one layer up.
        raise EditorError(
            gettext(
                "Opening an editor needs an interactive terminal — not a pipe, CI, or "
                "--no-input. Edit the file directly instead: %(path)s"
            )
            % {"path": str(path)}
        )
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


def edit_draft_path(slug: str, suffix: str) -> Path:
    """A unique staging path for one editor session, in skit's own drafts dir (kept on
    refusal, like every draft — "your edit was kept" must be a promise the OS can't
    break, which $TMPDIR isn't). NEVER the skit- prefix: that names the add flow's own
    drafts, and an edit's scratch file must not surface in its resume list."""
    from .paths import drafts_dir

    directory = drafts_dir()
    directory.mkdir(parents=True, exist_ok=True)
    fd, raw = tempfile.mkstemp(prefix=f"edit-{slug}-", suffix=suffix, dir=directory)
    os.close(fd)
    return Path(raw)


def stale_edit_kept(error: str, draft: Path) -> str:
    """The stale-edit refusal plus the recovery path: the session's work is IN the
    draft, and saying so is the difference between a refusal and a data loss."""
    return gettext("%(error)s Your edit was kept at: %(path)s") % {
        "error": error,
        "path": str(draft),
    }


def discard_draft(draft: Path) -> None:
    """Best-effort cleanup of a finished session's draft: a draft that cannot be
    deleted (a Windows handle still open on it) is harmless litter in a dir the user
    can see and manage — failing the edit over it would be backwards."""
    with contextlib.suppress(OSError):  # pragma: no mutate — narrowing the suppress is only observable with an undeletable file, which no portable test can stage  # fmt: skip
        draft.unlink()


def edit_copy_staged(source: Path, draft: Path, *, kind: str) -> bytes | None:
    """Edit a STORED COPY through a staged draft — the editor never sees the stored
    path. An editor session is the longest user-paced hold skit has: editing the real
    path directly would let a save land on whatever entry owns that path by the time
    the user writes (a remove + same-name re-add rebuilds it), and no post-hoc check
    can un-write it. The draft lives in skit's own drafts dir (kept on refusal, like
    every draft), the kind-specific payload validation runs against the DRAFT — so a
    refused prompt edit never lands replacement characters on the stored copy either —
    and the caller commits the returned bytes through store.commit_copy_edit's
    identity-checked transaction. None = the editor left the draft byte-identical
    (nothing to commit; the draft is cleaned up)."""
    shutil.copy2(source, draft)  # the draft file itself was just minted (edit_draft_path)
    staged = draft.read_bytes()
    open_entry_in_editor(draft, kind=kind)  # validation refusals keep the draft
    edited = draft.read_bytes()
    if edited == staged:
        discard_draft(draft)
        return None
    return edited


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
