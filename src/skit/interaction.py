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
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from typing import TextIO

_forbidden = False


def forbid() -> None:
    """Record that this invocation may not prompt — ``--no-input``, or any caller that
    knows it has no user. One-way on purpose: nothing re-grants permission mid-run."""
    global _forbidden
    _forbidden = True


def reset() -> None:
    """Restore the default (ask the terminal).

    For TESTS, and only tests — the seam a process-global has to expose to be testable at
    all: one test's ``--no-input`` would otherwise silently suppress the next test's
    prompt, which is exactly what it did before tests/conftest.py called this. No product
    code calls it, and this docstring says so rather than inventing a caller: a line
    describing behaviour nobody has is the shape LangSpec.takes_argv was deleted for.

    (``_forbidden = False`` has one equivalent mutant, ``= None``: both are falsy at the
    single place the flag is read. Left alone rather than contorting the read to defeat
    it.)"""
    global _forbidden
    _forbidden = False


def allowed(*, on: TextIO | None = None) -> bool:
    """Whether a prompt is permissible right now.

    ``--no-input`` wins outright. Otherwise BOTH ends of the conversation must be a
    terminal — stdin to answer with, and the stream the question is printed to, or the
    user is asked something they cannot see.

    `on` is that stream, because skit has two answering surfaces and they are not
    interchangeable: cli.py's `Prompt.ask` writes to **stdout**, uvman's download consent
    to **stderr** (stdout there belongs to the launched script). Asking one question about
    the other's stream is how this module briefly disagreed with `cli._is_interactive`
    in both directions at once — `skit run x > out` had cli decline to prompt while uvman
    blocked on one, and `skit run x 2> log` had cli open a form while uvman silently
    downloaded and executed a network binary with no consent at all. One oracle, told
    which stream it is answering about.
    """
    if _forbidden or not sys.stdin.isatty():
        return False
    return (on if on is not None else sys.stdout).isatty()
