"""Frozen-binary entry point (PyInstaller only — installed wheels use the console script).

multiprocessing.freeze_support() is insurance, not a current need: skit itself never uses
multiprocessing, but in a frozen app sys.executable IS this binary, so any future dependency
spawning a worker would re-exec skit in an infinite loop without it. The call is a no-op in
every process except a multiprocessing child.
"""

import multiprocessing

from skit.cli import app

if __name__ == "__main__":
    multiprocessing.freeze_support()
    app()
