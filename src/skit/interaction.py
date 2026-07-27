"""May skit ask the user anything at all? ONE answer, readable from every layer.

The non-interactive contract (AGENTS.md principle 4) is absolute: in a pipe, in CI, or
under ``--no-input``, skit never prompts and never guesses. It was enforced as a *local
variable*: ``cli.py`` threaded its ``no_input`` flag through its own gates and stopped
there. Everything below — ``flows.execute`` → ``launcher.run_entry`` →
``UvLaunch.build`` → ``ensure_uv`` → ``uvman._ask_consent`` — re-derived interactivity
from ``sys.stdin.isatty()``, an oracle the flag cannot reach. So ``skit run x
--no-input`` on a machine without uv printed a consent question and blocked on
``input()`` forever, which is precisely what the bundled Agent Skill promises agents
cannot happen.

Threading a ``quiet=`` keyword down that call chain would have fixed the one gate that
exists today; the next gate added below ``cli.py`` would repeat the bug. So the verdict
lives here instead, set once at the front door and asked by whoever needs it, at any
depth, without a parameter to forget.

Deliberately process-global: it describes THIS invocation's terminal, which no argument
can vary within a run. Headless, stdlib-only.
"""

from __future__ import annotations

import sys

_forbidden = False


def forbid() -> None:
    """Record that this invocation may not prompt — ``--no-input``, or any caller that
    knows it has no user. One-way on purpose: nothing re-grants permission mid-run."""
    global _forbidden
    _forbidden = True


def reset() -> None:
    """Restore the default (ask the terminal). For tests and for the TUI's in-process
    re-entry into CLI code paths, which must not inherit a previous command's refusal."""
    global _forbidden
    _forbidden = False


def allowed() -> bool:
    """Whether a prompt is permissible right now.

    ``--no-input`` wins outright. Otherwise both ends of the conversation must be a
    terminal: skit asks on stderr (stdout belongs to the launched script), so a piped
    stderr means the question would never be seen even though stdin could answer it.
    """
    return not _forbidden and sys.stdin.isatty() and sys.stderr.isatty()
