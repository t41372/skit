"""The console-script entry point: a dispatcher thin enough to answer `--version`
without building the CLI.

`skit.cli` is a 5000-line Typer app whose decorators run at import time, so importing
it costs typer, rich and every module the command bodies close over — ~230 modules
before a single argument is parsed. Most of that is unavoidable once a real command
runs. `--version` is not a real command: it answers from package metadata alone, and
agents and packaging checks call it far more often than people do.

So the version flag is answered here, ahead of the import, and everything else falls
through to Typer unchanged. The fast path only claims the flag in FIRST position, which
is the only place Typer itself accepts it as an app-level option — anywhere else it is
either a subcommand's own flag or a usage error, and both of those answers belong to
Typer, not to this file.
"""

from __future__ import annotations

import sys

_VERSION_FLAGS = ("--version", "-V")


def main() -> None:
    if sys.argv[1:2] and sys.argv[1] in _VERSION_FLAGS:
        from . import __version__

        # Plain print, not the CLI's rich Console: `--version` is a machine-facing
        # answer, and rich's number highlighter splits a PEP 440 version into colored
        # fragments ("0.4" cyan, then ".", then "1." cyan) on a terminal.
        print(f"skit {__version__}")
        return
    from .cli import app

    app()


if __name__ == "__main__":
    main()
