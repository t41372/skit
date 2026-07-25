"""skit — a launcher and parameter manager for your scripts."""

from __future__ import annotations

import os
from typing import TYPE_CHECKING

# Textual ≥ 8.2.7 enables the kitty keyboard protocol's "report all keys" mode, which
# iTerm2 (3.6.x) implements in a way that fights the macOS IME: candidate-selection
# digits and Enter reach the app as raw key events and the composed CJK text is never
# delivered at all (iTerm2 issue 12906) — Chinese/Japanese/Korean typing breaks. No
# skit binding needs the protocol, so opt out before the first textual import
# (textual.constants reads this at import time). setdefault keeps an explicit user
# override (=0 re-enables) winning.
os.environ.setdefault("TEXTUAL_DISABLE_KITTY_KEY", "1")

if TYPE_CHECKING:  # resolved at runtime by __getattr__ below
    __version__: str


def __getattr__(name: str) -> str:
    """Resolve ``skit.__version__`` on first access (PEP 562).

    pyproject.toml is the single source of the version; installed distributions carry
    it as metadata (a wheel doesn't ship pyproject.toml), so read it from there — but
    `importlib.metadata` drags in the whole `email` parser stack, ~85 modules, which
    is the single largest import cost on skit's startup path. Almost no invocation
    needs the version, so nothing pays for it until something asks. The resolved value
    is cached into the module globals, so this runs at most once per interpreter (and
    `importlib.reload(skit)` clears it, which is how the no-distribution fallback stays
    testable).
    """
    if name != "__version__":
        raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
    from importlib.metadata import PackageNotFoundError, version

    try:
        resolved = version("skit-cli")
    except PackageNotFoundError:  # a bare checkout on sys.path, no installed dist
        resolved = "0.0.0+unknown"
    globals()["__version__"] = resolved
    return resolved
