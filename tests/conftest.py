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
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

# Import-time scrub, deliberately not a fixture: skit.cli's module-level rich Console
# instances are constructed during collection — before any fixture runs — and they read
# FORCE_COLOR/NO_COLOR at construction time. A shell exporting FORCE_COLOR (observed:
# FORCE_COLOR=3) otherwise repaints every exact-output assertion with ANSI codes.
for _var in ("FORCE_COLOR", "NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE"):
    os.environ.pop(_var, None)

from skit import (  # noqa: E402 — must import after the color scrub above
    i18n,
    interaction,
    tui_footer,
)

if TYPE_CHECKING:
    from collections.abc import Iterator

    from textual.widgets import Static

    from skit.store import Entry

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

    # Hermetic console width: skit's CLI output is a piped, non-tty rich.Console under CliRunner,
    # so rich reads COLUMNS (falling back to 80). A test that asserts a substring living inside a
    # long absolute path (a tmp_path, an entry dir) then passes or fails purely on how long the
    # temp directory happens to be that run — rich soft-wraps or middle-truncates the path token
    # ("script.sh" -> "s\ncript.sh", "[red]weird[bold]" -> "[red]w...py") once it overflows the
    # width. Pin a wide width so those assertions test what they mean to, deterministically, no
    # matter how deep the tmp path is (pytest's counter climbs across a session and under mutmut).
    monkeypatch.setenv("COLUMNS", "200")

    # Hermetic terminal capabilities: several CLI tests force `_is_interactive()` true to
    # exercise the Textual form path.  Their result must not depend on whether the shell that
    # launched pytest exported TERM=dumb (which deliberately selects the plain fallback in
    # production).  Tests for that fallback set TERM=dumb explicitly after this fixture runs.
    monkeypatch.setenv("TERM", "xterm-256color")

    # Hermetic color, second layer: the import-time scrub above already cleaned the
    # process env; this keeps any subprocess a test spawns clean too, even if a test
    # setenv'd something exotic in between.
    for var in ("FORCE_COLOR", "NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE"):
        monkeypatch.delenv(var, raising=False)


def full_mirror():
    """All three mirror axes on their recommended presets — the standard "mirrors on"
    fixture for tests that just need an enabled mirror config. Axes stay independently
    settable in prod; only tests bundle them for convenience."""
    from skit import config  # deferred: see the import-order note above

    return config.compose(
        pypi=config.PYPI_PRESETS["tsinghua"],
        python_install=config.PYTHON_INSTALL_MIRROR,
        uv_binary=config.UV_BINARY_MIRROR,
        npm=config.NPM_REGISTRY_MIRROR,
    )


_BLOCK_OPEN = b"# /// script"
_BLOCK_CLOSE = b"# ///"


def without_block(raw: bytes, newline: bytes) -> bytes:
    """Drop an inserted `# /// script` … `# ///` comment block, keeping every other byte —
    terminators included — exactly where it lies, so a comparison against the original bytes
    is a real byte-for-byte claim about the rest of the file rather than a normalized diff.

    THE shared copy: the block-edit write-back is asserted from both the helper level
    (tests/test_design_audit_fixes.py) and the screen level (tests/test_design_audit_tui.py),
    and the two must not drift into two different notions of "everything else"."""
    keep: list[bytes] = []
    inside = False
    for chunk in raw.split(newline):
        if chunk == _BLOCK_OPEN:
            inside = True
            continue
        if inside:
            inside = chunk != _BLOCK_CLOSE
            continue
        keep.append(chunk)
    return newline.join(keep)


def plan_cache_key(entry: Entry) -> tuple[int, int, int, int, str | None]:
    """The MenuApp._plan_cache key for `entry`: (mtime_ns, size) of the stored script AND of
    its meta.toml — a display plan is a function of both files — plus the kind's reader-tool
    fingerprint, because a reader plan (PowerShell) is a function of pwsh availability too.
    mtime_ns + size (not a float mtime) narrows the same-tick blind spot on coarse-mtime
    filesystems to same-tick same-size writes. Shared so the key's shape is spelled out in
    exactly one place."""
    from skit.langs.registry import spec_for  # deferred: see the import-order note above

    script = entry.script_path.stat()
    meta = (entry.dir / "meta.toml").stat()
    spec = spec_for(entry.meta.kind)
    reader = spec.cli_reader if spec is not None else None
    fingerprint = reader.runtime_fingerprint if reader is not None else None
    tool = fingerprint() if fingerprint is not None else None
    return (script.st_mtime_ns, script.st_size, meta.st_mtime_ns, meta.st_size, tool)


def real_repo_root() -> Path:
    """The repository root, even inside mutmut's mutants/ copy.

    Meta-tests that read skit's SOURCE (AST walks, structural asserts, the blind-spot
    measure) must read the REAL tree: inside mutants/ every undecorated function has
    been trampoline-rewritten, so the copy describes mutmut's machinery, not the code
    under test — and the ratchets those tests enforce would trip on the rewrite itself.
    The strip-the-prefix idiom is round 10's `_gate_module`, shared here so every
    source-reading test resolves the same way."""
    root = Path(__file__).resolve().parent.parent
    if "mutants" in root.parts:
        root = Path(*root.parts[: root.parts.index("mutants")])
    return root


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


@pytest.fixture
def at_a_terminal(monkeypatch: pytest.MonkeyPatch) -> None:
    """Declare that this test sits at a real terminal.

    Tests are non-interactive by default (CliRunner's stdin is not a tty), which is the
    right default — it is what a pipe, CI and an agent all look like. Any test that drives
    a lane skit will only run for a human (an editor session, an interactive prompt) has
    to say so, because since round 11/12 those lanes REFUSE rather than blocking on a
    stdin nobody is typing into."""
    # Patch the CLI's own name for the question, not sys.*.isatty: CliRunner replaces the
    # standard streams for the duration of invoke(), so a patched isatty on the outer
    # objects never reaches the command. This is the seam 176 existing tests already use.
    from skit import cli

    monkeypatch.setattr(cli, "_is_interactive", lambda: True)


@pytest.fixture(autouse=True)
def _reset_interaction() -> None:
    """The non-interactive verdict is a process-global, like the locale below: a CLI test
    that passes --no-input calls interaction.forbid(), and without this the NEXT test's
    consent prompt would be silently suppressed by a flag it never set. (Same reason the
    product exposes reset(): the TUI re-enters CLI code paths in-process.)"""
    interaction.reset()


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
def _sweep_injected_temp_copies(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    """Delete the injected temp copies THIS test created, and nothing else.

    The product path always unlinks its own temp copy (flows.execute's `finally`), but the injector
    tests call `inject()` directly and get a real 0600 file back — one that can carry a plaintext
    secret literal from a test fixture. Nothing else would ever remove them, so an unswept suite
    quietly accumulates thousands of secret-bearing files in the OS temp directory.

    The paths are captured by wrapping `write_injected` itself, NOT by diffing a glob of the temp
    directory: a glob-diff deletes every `.injected-*` that appeared during the test whoever made
    it, so a developer running the suite while using skit in another terminal — or a second xdist
    worker — would have its live temp copy pulled out from under it between write and exec. Each
    injector binds `write_injected` at import, so every importing namespace is patched.
    """
    from skit import rewrite
    from skit.langs.javascript import inject as js_inject
    from skit.langs.python import shim as py_shim
    from skit.langs.shell import inject as sh_inject

    created: list[Path] = []
    real = rewrite.write_injected

    def tracking(
        entry_dir: Path, content: str, *, suffix: str, prefer_entry_dir: bool = False
    ) -> Path:
        path = real(entry_dir, content, suffix=suffix, prefer_entry_dir=prefer_entry_dir)
        created.append(path)
        return path

    for module in (rewrite, js_inject, py_shim, sh_inject):
        monkeypatch.setattr(module, "write_injected", tracking)
    yield
    for path in created:
        with contextlib.suppress(OSError):
            path.unlink()
