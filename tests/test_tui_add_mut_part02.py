"""Mutation-kill tests for AddReviewScreen._compose_deps (tui_add.py chunk 2/5).

Every test drives the real review panel through a Textual `Pilot` and pins an OBSERVABLE
contract of the Dependencies section: the exact label text, the CSS class each label
carries, the read-only PEP 723 lines, the suggested-deps Input value/placeholder, and the
"(none declared)" markup rendering. The assertions use EXACT per-Static renders (not
substring `in`) because the surviving mutants wrap strings in `XX…XX` — a substring check
passes right through them.
"""

from __future__ import annotations

import pytest
from textual.widgets import Input, Static

from skit import tui
from skit.tui_add import AddReviewScreen


@pytest.fixture(autouse=True)
def _en(monkeypatch):
    # English is the msgid source, so gettext returns the msgids verbatim — the exact-string
    # assertions below depend on that. conftest already pins SKIT_LANG=en; make it explicit.
    monkeypatch.setenv("SKIT_LANG", "en")


def _py(tmp_path, body: str, name: str = "job.py"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _renders(screen) -> list[str]:
    """The exact rendered text of every Static on the screen (RadioButton/Checkbox ride
    along as Static subclasses)."""
    return [str(w.render()) for w in screen.query(Static)]


def _has_static(screen, text: str, cls: str) -> bool:
    """True iff some Static renders EXACTLY `text` and carries CSS class `cls`."""
    return any(str(w.render()) == text and w.has_class(cls) for w in screen.query(Static))


async def test_dependencies_section_label(tmp_path):
    """The section header reads exactly 'Dependencies' and wears the `.section` class —
    both the text and the class are load-bearing (the class drives the accent styling)."""
    p = _py(tmp_path, "print(1)\n")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        assert _has_static(screen, "Dependencies", "section")


async def test_pep723_python_and_deps_lines_are_exact(tmp_path):
    """A declared PEP 723 block renders the read-only header plus one '· needs Python …'
    line and one '· installs …' line per dependency — verbatim, bullet included."""
    src = (
        "# /// script\n"
        '# requires-python = ">=3.11"\n'
        '# dependencies = ["requests"]\n'
        "# ///\n"
        "print(1)\n"
    )
    p = _py(tmp_path, src, "declared.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        renders = _renders(screen)
        assert "The script declares its own dependencies (PEP 723):" in renders
        assert "· needs Python >=3.11" in renders
        assert "· installs requests" in renders


async def test_declared_deps_without_python_do_not_show_none_declared(tmp_path):
    """`if not deps and not python` guards the '(none declared)' line: with deps present
    (but no requires-python) it must NOT appear. The `and`->`or` mutant would show it."""
    src = '# /// script\n# dependencies = ["requests"]\n# ///\nprint(1)\n'
    p = _py(tmp_path, src, "deps_no_py.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        renders = _renders(screen)
        assert "· installs requests" in renders  # we are in the declared-deps branch
        assert "(none declared)" not in renders


async def test_empty_declared_block_says_none_declared_with_markup(tmp_path):
    """An empty declared block renders '(none declared)' through Rich markup: the label
    shows the styled text, never the literal '[dim]…[/dim]' brackets."""
    src = "# /// script\n# dependencies = []\n# ///\nprint(1)\n"
    p = _py(tmp_path, src, "empty.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        renders = _renders(screen)
        assert "(none declared)" in renders  # exact text, markup applied
        assert "[dim](none declared)[/dim]" not in renders  # markup NOT disabled


async def test_suggested_deps_input_value_and_placeholder(tmp_path):
    """A script with third-party imports and no PEP 723 block gets an editable deps Input
    prefilled with the comma-joined suggestions and the guidance placeholder."""
    src = "import requests\nimport rich\nprint(1)\n"
    p = _py(tmp_path, src, "imports.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        field = screen.query_one("#rv-deps", Input)
        assert field.value == "requests, rich"
        assert field.placeholder == "comma separated, e.g. requests>=2,<3, rich"


async def test_deps_hint_label(tmp_path):
    """Below the editable deps Input, the hint reads exactly 'detected from the script's
    imports — edit freely' and carries the `.hint` class."""
    src = "import requests\nprint(1)\n"
    p = _py(tmp_path, src, "hint.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        assert _has_static(screen, "detected from the script's imports — edit freely", "hint")


# ---------------------------------------------------------------------------
# npm-flavor deps: the #rv-deps Input placeholder + the detected-imports hint
# ---------------------------------------------------------------------------


async def test_npm_deps_input_placeholder_and_hint(tmp_path):
    """A js (npm-flavor) add renders the deps Input with the npm example placeholder and the
    detected-imports hint below it — the npm branch's own copy, verbatim and classed .hint."""
    p = _py(tmp_path, "const x = 1\n", "w.js")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p, kind="js")
        app.push_screen(screen)
        await pilot.pause()
        field = screen.query_one("#rv-deps", Input)
        assert field.placeholder == "comma separated, e.g. chalk, @scope/pkg"
        assert _has_static(screen, "detected from the script's imports — edit freely", "hint")


# ---------------------------------------------------------------------------
# uv-flavor requires-python: the #rv-python Input placeholder + its guidance hint
# ---------------------------------------------------------------------------


async def test_uv_requires_python_input_placeholder_and_hint(tmp_path):
    """A python add with no PEP 723 block mounts the editable #rv-python field: its placeholder
    reads '(automatic)' and the guidance hint below it explains the #!-line prefill — both
    verbatim, the hint classed .hint."""
    p = _py(tmp_path, "print(1)\n", "s.py")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = AddReviewScreen(p)
        app.push_screen(screen)
        await pilot.pause()
        field = screen.query_one("#rv-python", Input)
        assert field.placeholder == "(automatic)"
        assert _has_static(
            screen,
            "Python version (requires-python) — prefilled from the #! line when "
            "it pins one; empty means automatic",
            "hint",
        )
