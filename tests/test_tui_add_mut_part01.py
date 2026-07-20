"""Mutation-kill tests for src/skit/tui_add.py (chunk 1/5).

Targets the CLI-facing entry points of the add-review panel — run_add_review,
AddReviewApp and AddReviewScreen constructors — pinning that every `skit add` flag is
forwarded, undisturbed, all the way to the screen's prefill overrides, and that the
script text is read leniently (utf-8 + errors="replace").

These exercise real constructor / forwarding behaviour: the panel prefill is exactly
what a terminal `skit add x.py --name … --link --deps …` depends on, and the lenient
read is what lets a script with a stray non-UTF-8 byte still reach the panel instead of
crashing the wizard.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import tui_add


def _py(tmp_path: Path, body: str = "x = 1\n", name: str = "tool.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# run_add_review — the blocking CLI entry forwards every kwarg to the screen
# ---------------------------------------------------------------------------


def test_run_add_review_forwards_every_arg_to_the_screen(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """`skit add` passes its flags through run_add_review → AddReviewApp → AddReviewScreen.
    Each value must land in the panel's prefill overrides; a dropped or nulled kwarg would
    silently ignore what the user asked for. Only the blocking event loop (.run()) is
    stubbed — the real App/Screen constructors run and build the real override state."""
    captured: dict[str, tui_add.AddReviewApp] = {}

    def fake_run(self: tui_add.AddReviewApp) -> str:
        captured["app"] = self
        return "the-slug"

    monkeypatch.setattr(tui_add.AddReviewApp, "run", fake_run)
    p = _py(tmp_path)

    result = tui_add.run_add_review(
        p,
        name="chosen-name",
        description="chosen desc",
        reference=True,
        deps=["requests", "rich"],
        requires_python=">=3.11",
    )

    assert result == "the-slug"  # .run()'s slug is returned untouched
    screen = captured["app"]._screen
    assert isinstance(screen, tui_add.AddReviewScreen)  # the host generalized in #14
    assert screen._overrides["name"] == "chosen-name"
    assert screen._overrides["desc"] == "chosen desc"
    assert screen._overrides["mode"] == "1"  # reference=True → the "link the original" radio
    assert screen._overrides["deps"] == "requests, rich"
    assert screen._requires_python == ">=3.11"


def test_run_add_review_defaults_to_copy_mode_and_no_python_pin(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Called with only a path, the defaults must be reference=False (copy — no mode
    override, so the panel boots on "keep a copy") and requires_python="" (no pin)."""
    captured: dict[str, tui_add.AddReviewApp] = {}

    def fake_run(self: tui_add.AddReviewApp) -> None:
        captured["app"] = self

    monkeypatch.setattr(tui_add.AddReviewApp, "run", fake_run)
    p = _py(tmp_path)

    tui_add.run_add_review(p)

    screen = captured["app"]._screen
    assert isinstance(screen, tui_add.AddReviewScreen)  # the host generalized in #14
    assert "mode" not in screen._overrides  # reference defaults False → no override
    assert screen._requires_python == ""  # requires_python defaults to the empty string


# ---------------------------------------------------------------------------
# AddReviewApp / AddReviewScreen — constructor default for requires_python
# ---------------------------------------------------------------------------


def test_add_review_app_defaults_requires_python_to_empty(tmp_path: Path) -> None:
    """AddReviewApp() with no requires_python builds a screen carrying "" — a non-empty
    default would pin every interactive add to a phantom Python version."""
    app = tui_add.AddReviewApp(_py(tmp_path))
    screen = app._screen
    assert isinstance(screen, tui_add.AddReviewScreen)  # the host generalized in #14
    assert screen._requires_python == ""


def test_add_review_screen_defaults_requires_python_to_empty(tmp_path: Path) -> None:
    """AddReviewScreen() with no requires_python stores "" (the value handed to
    store.add_python on accept)."""
    screen = tui_add.AddReviewScreen(_py(tmp_path))
    assert screen._requires_python == ""


# ---------------------------------------------------------------------------
# AddReviewScreen — lenient read of the script text (errors="replace")
# ---------------------------------------------------------------------------


def test_add_review_screen_reads_invalid_utf8_with_replace(tmp_path: Path) -> None:
    """The panel must open even for a script with a stray non-UTF-8 byte. errors="replace"
    turns the bad bytes into U+FFFD and never raises; a strict decode (errors=None or a
    dropped errors kwarg) raises UnicodeDecodeError, and a bogus handler name ("REPLACE" /
    "XXreplaceXX") raises LookupError — any of which would crash AddReviewScreen.__init__."""
    p = tmp_path / "weird.py"
    p.write_bytes(b"x = 1  # \xff\xfe tail\n")

    screen = tui_add.AddReviewScreen(p)  # must not raise

    assert "�" in screen._text  # the invalid bytes were replaced, not decoded
    assert screen._text.startswith("x = 1")  # the valid prefix survived intact


# ---------------------------------------------------------------------------
# AddReviewScreen — deps override is comma-space joined
# ---------------------------------------------------------------------------


def test_add_review_screen_joins_deps_override_with_comma_space(tmp_path: Path) -> None:
    """A `--deps requests rich` prefill is rendered into the single deps Input as one
    ", "-joined string; a different separator would corrupt the package list the user sees
    (and edits)."""
    screen = tui_add.AddReviewScreen(_py(tmp_path), deps=["requests", "rich"])
    assert screen._overrides["deps"] == "requests, rich"


# ---------------------------------------------------------------------------
# AddReviewScreen.__init__ — the read-error fallback (_text / _text_error)
# ---------------------------------------------------------------------------


def test_init_clean_read_leaves_no_text_error(tmp_path: Path) -> None:
    """A source that reads cleanly leaves ``_text_error`` as the empty string, never None: the
    panel opens in the no-error state (the ``= ""`` init vs the ``= None`` mutant)."""
    screen = tui_add.AddReviewScreen(_py(tmp_path))
    assert screen._text_error == ""


def test_init_unreadable_source_records_the_read_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """When the source can't be read, __init__ degrades ``_text`` to the empty string (not
    "XXXX") and records the exact 'Can't read <path>: <error>' message. Forcing a strerror-less
    OSError('boom') pins that the message text is ``exc.strerror or str(exc)`` == 'boom' (the
    ``or``->``and`` and ``str(None)`` mutants both yield 'None'), plus the msgid and its case."""
    p = _py(tmp_path)

    def boom(self: Path, *a: object, **k: object) -> str:
        raise OSError("boom")  # no errno/strerror → str(exc) == "boom"

    monkeypatch.setattr(Path, "read_text", boom)
    screen = tui_add.AddReviewScreen(p)  # must not raise
    assert screen._text == ""  # the empty fallback, not "XXXX"
    assert screen._text_error == f"Can't read {p}: boom"


def test_init_read_error_uses_strerror_not_the_errno_repr(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """With a real errno-bearing OSError the message uses ``exc.strerror`` (the clean phrase),
    never ``str(exc)`` (which carries the '[Errno N]' prefix) — the ``or``->``and`` mutant would
    swap to the latter."""
    p = _py(tmp_path)

    def boom(self: Path, *a: object, **k: object) -> str:
        raise OSError(21, "Is a directory")

    monkeypatch.setattr(Path, "read_text", boom)
    screen = tui_add.AddReviewScreen(p)
    assert screen._text_error == f"Can't read {p}: Is a directory"
    assert "[Errno" not in screen._text_error  # str(exc)'s errno prefix must not leak in
