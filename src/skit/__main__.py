"""The console-script entry point: a dispatcher thin enough to answer `--version`
without building the CLI.

`skit.cli` is a 5000-line Typer app whose decorators run at import time, so importing
it costs typer, rich and every module the command bodies close over — ~230 modules
before a single argument is parsed. Most of that is unavoidable once a real command
runs. `--version` is not a real command: it answers from package metadata alone, and
agents and packaging checks call it far more often than people do.

So the version flag is answered here, ahead of the import, and everything else falls
through to Typer unchanged. The fast path claims the flag only when it is the WHOLE
command line — `skit --version`, nothing else. That is the one invocation whose answer
cannot depend on anything Typer would have parsed: `skit --version foo` is a usage
error Typer reports ("No such command 'foo'"), and `skit --version list` prints the
version through the callback. Claiming a leading flag and ignoring the rest of argv
would turn both of those into a silent exit 0.
"""

from __future__ import annotations

import sys

_VERSION_ARGV = (["--version"], ["-V"])


def print_version() -> None:
    """The one spelling of the version line, shared with the CLI callback.

    Plain print, not the CLI's rich Console: `--version` is a machine-facing answer,
    and rich's number highlighter splits a PEP 440 version into colored fragments
    ("0.4" cyan, then ".", then "1." cyan) on a terminal. Both paths that answer the
    flag call THIS — an agent parsing the output must get one answer whatever the
    argv shape, and two hand-synced f-strings would drift silently.
    """
    from . import __version__

    print(f"skit {__version__}")


def main() -> None:
    if sys.argv[1:] in _VERSION_ARGV:
        print_version()
        return
    from .cli import app

    app()


if __name__ == "__main__":
    main()
