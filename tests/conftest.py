"""Shared test fixtures.

Tests must be hermetic: they must never read from or write to the developer's real
skit config/data/state directories (e.g. ~/Library/Application Support/skit on macOS).
src/skit/paths.py resolves each directory from SKIT_CONFIG_DIR / SKIT_DATA_DIR /
SKIT_STATE_DIR (falling back to platformdirs, i.e. the real user directories, when
unset), read live on every call. Without isolation, a test that doesn't explicitly
monkeypatch these env vars will silently fall through to the real config — this is
exactly what happened with a real ~/.../skit/config.toml containing `language =
"zh-TW"`, which made tests/test_i18n.py::test_lang_env and
tests/test_review_fixes.py::test_detect_locale_locale_module_error fail (and, in an
earlier run, caused a first-run locale auto-detect to write to that real file).

This autouse fixture points all three env vars at a per-test tmp_path subdirectory
before every test runs, so the real user directories are never touched regardless of
whether a given test's own fixtures also set them. Per-file/per-test monkeypatching
of these vars still works fine on top of this (monkeypatch composes; the last set
wins) — this is additive, not a replacement for it.

Second layer (mutation-escape hardening): mutation testing (`uv run mutmut run`)
mutates src/skit/paths.py itself — e.g. corrupting the "SKIT_DATA_DIR" string literal
used in os.environ.get(). When that happens, the SKIT_* lookup above silently misses
and paths.py falls through to its platformdirs-based default, which resolves against
the real ~/Library/Application Support/skit (macOS) or ~/.local/share (Linux). The
mutant still gets killed by an assertion elsewhere, but by then the test has already
written ghost files into the developer's real registry — this actually happened once,
clobbering a real registry entry with pytest tmp-path junk. Env-var isolation alone is
structurally insufficient when the isolation-implementing code is the thing being
mutated. So this fixture also redirects the fallback layer: platformdirs on macOS
resolves user_data_dir/user_state_dir/user_config_dir via HOME (empirically verified:
overriding HOME repoints all three under "<HOME>/Library/Application Support"), and on
Linux via XDG_DATA_HOME/XDG_STATE_HOME/XDG_CONFIG_HOME (falling back to HOME-relative
defaults otherwise). Redirecting HOME plus the XDG_* vars means that even if every
SKIT_* lookup in paths.py were deleted or broken by a mutant, platformdirs would still
resolve inside tmp_path, never the real user directories.

Corollary for humans and agents: never run the real `uv run skit` CLI for manual
testing/debugging without first pointing SKIT_CONFIG_DIR/SKIT_DATA_DIR/SKIT_STATE_DIR
(or HOME) at a scratch directory — that was the other source of real-directory
pollution, independent of the test suite.
"""

from __future__ import annotations

import contextlib
import os
import tempfile
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

# Import-time scrub, deliberately not a fixture: skit.cli's module-level rich Console
# instances are constructed during collection — before any fixture runs — and they read
# FORCE_COLOR/NO_COLOR at construction time. A shell exporting FORCE_COLOR (observed:
# FORCE_COLOR=3) otherwise repaints every exact-output assertion with ANSI codes.
for _var in ("FORCE_COLOR", "NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE"):
    os.environ.pop(_var, None)

from skit import i18n, tui_footer  # noqa: E402 — must import after the color scrub above

if TYPE_CHECKING:
    from collections.abc import Iterator

    from textual.widgets import Static

# NOTE: textual must NOT be imported at conftest top level. pytest loads conftest before
# any test module, which makes it the process's first importer: skit/__init__ has to run
# before textual.constants reads TEXTUAL_DISABLE_KITTY_KEY from the environment
# (tests/test_ime_input.py pins exactly this ordering).


@pytest.fixture(autouse=True)
def _isolate_skit_dirs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))

    # Defense in depth: also redirect the platformdirs fallback layer itself, in case
    # a mutant breaks the SKIT_* lookups above (see module docstring).
    fake_home = tmp_path / "home"
    monkeypatch.setenv("HOME", str(fake_home))
    # HOME governs the home dir on POSIX; Windows reads USERPROFILE (os.path.expanduser / Path.home
    # consult it first), so redirect both or the fallback layer escapes to the real home on Windows.
    monkeypatch.setenv("USERPROFILE", str(fake_home))
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "xdg-data"))
    monkeypatch.setenv("XDG_STATE_HOME", str(tmp_path / "xdg-state"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "xdg-config"))

    # Hermetic locale: default every test to English so exact-message assertions never depend on
    # the developer's / CI host's LC_ALL/LANG (a zh host locale otherwise translates launcher and
    # i18n messages and breaks those assertions). Tests that exercise a specific locale override
    # this with their own monkeypatch.setenv("SKIT_LANG", ...) / delenv (same monkeypatch instance,
    # last-set wins), and the _reset_i18n fixture below clears the cached catalog so the override
    # takes effect. This replaces the fragile per-test/per-file SKIT_LANG pinning.
    monkeypatch.setenv("SKIT_LANG", "en")

    # Hermetic color, second layer: the import-time scrub above already cleaned the
    # process env; this keeps any subprocess a test spawns clean too, even if a test
    # setenv'd something exotic in between.
    for var in ("FORCE_COLOR", "NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE"):
        monkeypatch.delenv(var, raising=False)


def footer_text(static: Static) -> str:
    """Rendered footer text with the pill glue (U+2800, one cell wide like a space)
    normalized back to spaces, so label assertions and click offsets read naturally.
    THE shared copy — the glue scheme must change here and nowhere else."""
    return str(static.render()).replace(tui_footer.GLUE, " ")


async def click_label(pilot, selector: str, needle: str) -> None:
    """Click a footer chip by its visible key or label text (chips carry left padding
    of 1). Assumes the chip is on the footer's first rendered line — true at the wide
    sizes the nav/click tests run at."""
    from textual.widgets import Static  # deferred: see the import-order note above

    static = pilot.app.screen.query_one(selector, Static)
    plain = footer_text(static)
    idx = plain.find(needle)
    assert idx >= 0, (needle, plain)
    await pilot.click(selector, offset=(idx + 1, 0))
    await pilot.pause()


@pytest.fixture(autouse=True)
def _reset_i18n() -> None:
    # The i18n catalog is a lazy module-level singleton: import-time gettext() calls
    # (e.g. tui.py BINDINGS during collection) would otherwise lock the process to the
    # machine's locale before any test fixture runs, making English-string assertions
    # order-dependent. Reset so each test lazily re-inits from its own isolated env.
    i18n._translations = None
    i18n._active = i18n.DEFAULT_LOCALE
    i18n._pseudo = False


@pytest.fixture(autouse=True)
def _sweep_injected_temp_copies() -> Iterator[None]:
    """Delete any injected temp copy a test left behind in the OS temp directory.

    The product path always unlinks its own temp copy (flows.execute's `finally`), but the
    injector tests call `inject()` directly and get a real 0600 file back — one that can carry a
    plaintext secret literal from a test fixture. Nothing else would ever remove them, so an
    unswept suite quietly accumulates thousands of secret-bearing files in $TMPDIR.

    Only files that appeared DURING the test are removed, and only ones matching skit's own
    `.injected-*` prefix, so a concurrently-running skit (or another tool) is never touched.
    """
    tmp = Path(tempfile.gettempdir())
    before = set(tmp.glob(".injected-*"))
    yield
    for leaked in tmp.glob(".injected-*"):
        if leaked not in before:
            with contextlib.suppress(OSError):
                leaked.unlink()
