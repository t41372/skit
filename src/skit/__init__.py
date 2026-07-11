"""skit — a launcher and parameter manager for your scripts."""

import os

# Textual ≥ 8.2.7 enables the kitty keyboard protocol's "report all keys" mode, which
# iTerm2 (3.6.x) implements in a way that fights the macOS IME: candidate-selection
# digits and Enter reach the app as raw key events and the composed CJK text is never
# delivered at all (iTerm2 issue 12906) — Chinese/Japanese/Korean typing breaks. No
# skit binding needs the protocol, so opt out before the first textual import
# (textual.constants reads this at import time). setdefault keeps an explicit user
# override (=0 re-enables) winning.
os.environ.setdefault("TEXTUAL_DISABLE_KITTY_KEY", "1")

__version__ = "0.0.2.dev0"
