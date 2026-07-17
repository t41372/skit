"""Mutation-kill tests for AddReviewScreen._compose_params (tui_add.py, chunk 3/5).

Every test drives the real "add script" review panel through Textual's Pilot: it pushes an
AddReviewScreen over the live MenuApp for a script chosen to exercise one branch of
_compose_params, then asserts on the widgets that branch actually mounts — the exact
user-visible copy and the CSS class that styles each notice. The conftest fixtures pin the
English catalog and isolated skit dirs, so the message assertions read the English msgids
verbatim.
"""

from __future__ import annotations

from pathlib import Path

from textual.widgets import Checkbox, Static

from skit import tui
from skit.tui_add import AddReviewScreen


def _py(tmp_path: Path, body: str, name: str = "s.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _statics_text(screen: AddReviewScreen) -> str:
    """The concatenated rendered text of every Static in the review panel."""
    return "".join(str(w.render()) for w in screen.query(Static))


def _static_with(screen: AddReviewScreen, needle: str) -> Static:
    """The single Static whose rendered text contains `needle` (fails if none)."""
    for w in screen.query(Static):
        if needle in str(w.render()):
            return w
    raise AssertionError(f"no Static rendering {needle!r}")


async def _review(app: tui.MenuApp, pilot, path: Path) -> AddReviewScreen:
    screen = AddReviewScreen(path)
    app.push_screen(screen)
    await pilot.pause()
    return screen


# ---------------------------------------------------------------------------
# the "Parameters" section header (always yielded first)
# ---------------------------------------------------------------------------


async def test_parameters_header_text_and_section_class(tmp_path):
    """The panel always opens with a "Parameters" heading carrying the .section class that
    styles every section title. Pins the literal word and the class together."""
    p = _py(tmp_path, 'print("hello")\n')
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _review(app, pilot, p)
        header = next(w for w in screen.query(Static) if str(w.render()) == "Parameters")
        assert header.has_class("section")  # not "SECTION"/"XXsectionXX"/None


# ---------------------------------------------------------------------------
# uses_cli_framework, argparse modelled (spec.ok with fields)
# ---------------------------------------------------------------------------


async def test_cli_framework_readable_shows_field_count_notice(tmp_path):
    """argparse the panel could read statically: the success notice reports the field count
    and reassures the run form is generated."""
    p = _py(
        tmp_path,
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('--foo')\nap.parse_args()\n",
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _review(app, pilot, p)
        assert (
            "✓ skit read this script's own arguments (1 fields). Running it "
            "opens a form — nothing to memorize." in _statics_text(screen)
        )


# ---------------------------------------------------------------------------
# uses_cli_framework, spec NOT modellable (passthrough hint)
# ---------------------------------------------------------------------------


async def test_cli_framework_unmodellable_lists_frameworks_as_hint(tmp_path):
    """Two CLI frameworks skit can't model statically: the passthrough hint names both,
    comma-joined, and wears the .hint class."""
    p = _py(tmp_path, "import click\nimport docopt\nprint('x')\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _review(app, pilot, p)
        assert (
            "This script parses its own arguments (click, docopt); skit couldn't model "
            "them statically, so the run form offers a passthrough-arguments field."
            in _statics_text(screen)
        )
        # the comma separator between the two framework names is load-bearing copy
        assert "(click, docopt)" in _statics_text(screen)
        assert _static_with(screen, "passthrough-arguments").has_class("hint")


# ---------------------------------------------------------------------------
# candidate checkboxes: the "tick these" hint + an input() label
# ---------------------------------------------------------------------------


async def test_input_candidate_hint_and_numbered_label(tmp_path):
    """A bare input() prompt becomes a numbered, tick-able candidate under a .hint caption.
    Pins the caption, its class, and the 1-based call-number label."""
    p = _py(tmp_path, 'x = input("Name?")\nprint(x)\n')
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _review(app, pilot, p)
        tick = _static_with(screen, "Tick the ones")
        # exact match: a whole-msgid mutation ("XXTick…XX") must not slip past a substring check
        assert str(tick.render()) == "Tick the ones the run form should ask for:"
        assert tick.has_class("hint")
        box = screen.query_one(Checkbox)
        assert str(box.label) == "input() #1: 'Name?'"  # order 0 -> "#1"


# ---------------------------------------------------------------------------
# demoted candidate -> loop-accumulator warning
# ---------------------------------------------------------------------------


async def test_demoted_candidate_shows_accumulator_warning(tmp_path):
    """A constant mutated in a loop is demoted and flagged with a ⚠ warning line carrying
    the .warn class."""
    p = _py(tmp_path, "total = 0\nfor i in range(10):\n    total += i\nprint(total)\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _review(app, pilot, p)
        assert "  ⚠ looks like a loop accumulator — probably not a parameter" in _statics_text(
            screen
        )
        assert _static_with(screen, "loop accumulator").has_class("warn")


# ---------------------------------------------------------------------------
# filename literals -> "give it a name" tip
# ---------------------------------------------------------------------------


async def test_filename_literals_tip_names_each_literal(tmp_path):
    """Bare filename literals passed to calls get the 💡 "extract a named constant" tip,
    which repr-lists each literal comma-joined and wears the .hint class."""
    p = _py(tmp_path, "open('a.txt')\nopen('b.log')\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _review(app, pilot, p)
        assert (
            "💡 'a.txt', 'b.log' are written directly inside the code, so skit can't turn them "
            "into form fields. To manage one, first give it a name at the top of the "
            "script, e.g. OUTPUT = '…' (Ctrl+E edits it now)." in _statics_text(screen)
        )
        assert _static_with(screen, "written directly inside the code").has_class("hint")


# ---------------------------------------------------------------------------
# sys.argv -> passthrough info line
# ---------------------------------------------------------------------------


async def test_uses_argv_shows_extra_arguments_info(tmp_path):
    """A script reading sys.argv gets the info line (leading info glyph) about the
    extra-arguments field, carrying the .hint class."""
    p = _py(tmp_path, "import sys\nprint(sys.argv[1])\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _review(app, pilot, p)
        assert (
            "ℹ This script reads command-line arguments; the run form has an "  # noqa: RUF001
            "extra-arguments field for them." in _statics_text(screen)
        )
        assert _static_with(screen, "extra-arguments field").has_class("hint")
