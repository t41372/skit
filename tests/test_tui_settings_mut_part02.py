"""Mutation-kill tests for tui_settings.py (chunk 2/7): the declared-schema editor's
template/add-a-parameter surface, the Dependencies section (uv vs npm placeholders +
the Python-constraint field), and the Needs section.

Every assertion pins observable widget state a real user sees — the rendered section
header text, its CSS class, the prefilled Input value, and the placeholder copy — driven
through the live screen with a Textual Pilot. Widgets are located structurally (the
sibling immediately before a stable id) so a mutant that blanks or rewrites the header
text can't hide from the lookup.
"""

from __future__ import annotations

import pytest
from textual.widgets import Input, Static

from skit import i18n, store, tui
from skit.tui_settings import ScriptSettingsScreen


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")  # message assertions read the English catalog (msgid identity)


def _py(tmp_path, body: str, name: str = "job.py"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _exe(tmp_path, name: str = "prog"):
    tool = tmp_path / "mytool"
    tool.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    tool.chmod(0o755)
    return store.add_exe(tool, name=name)


def _body(screen) -> str:
    return " ".join(str(w.render()) for w in screen.query(Static))


def _prev_sibling(widget):
    """The widget composed immediately before `widget` among its parent's children.

    A structural (not text-based) locator: a mutant that blanks or rewrites a section
    header's text still leaves it sitting right before the stable-id Input below it.
    """
    kids = list(widget.parent.children)
    return kids[kids.index(widget) - 1]


# ---------------------------------------------------------------------------
# _compose_declared_editor: template line + the add-a-parameter hint & field
# ---------------------------------------------------------------------------


async def test_declared_editor_command_template_and_add_field(tmp_path):
    """A command (template family) entry renders its template EDITABLE (#14: the
    template IS the program — freezing it read-only forever while every other kind
    can edit its source was a rule with no reason), with the re-read hint's exact
    copy, and the add-a-parameter hint + input carry their exact copy, class, and
    placeholder."""
    entry = store.add_command("convert {size}", name="conv")
    async with tui.MenuApp().run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        pilot.app.push_screen(screen)
        await pilot.pause()

        # The template rides in the editable input, prefilled verbatim (kills value
        # None/drop), and the re-read hint keeps its exact copy and hint class.
        assert screen.query_one("#st-template", Input).value == "convert {size}"
        reread_hint = next(
            w for w in screen.query(Static) if "re-reads the {placeholders}" in str(w.render())
        )
        assert str(reread_hint.render()).strip() == (
            "Saving re-reads the {placeholders} from the template."
        )
        assert reread_hint.has_class("hint")

        add_input = screen.query_one("#st-add-param", Input)
        add_hint = _prev_sibling(add_input)
        assert isinstance(add_hint, Static)
        # Exact copy (kills XX-wrap / lower / upper / dropped-text mutants).
        assert str(add_hint.render()).strip() == "Add a parameter — type a name, then Save:"
        assert add_hint.has_class("hint")  # kills classes None / drop / XXhintXX / HINT
        # The add field's placeholder copy (kills dropped / XX-wrap / upper placeholder).
        assert add_input.placeholder == "new parameter name"


async def test_declared_editor_exe_has_no_template_line(tmp_path):
    """A non-template declared kind (exe) never renders the template line — the guard is an
    AND of "has a spec" AND "family is template", so an OR mutant would leak the template in.
    A sentinel template makes the (normally empty) field observable."""
    entry = _exe(tmp_path)
    entry.meta.template = "ZZZ-TEMPLATE-SENTINEL"  # exe entries carry no template; probe the guard
    async with tui.MenuApp().run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        pilot.app.push_screen(screen)
        await pilot.pause()
        assert screen.query("#st-add-param")  # the declared editor is what's showing
        assert "ZZZ-TEMPLATE-SENTINEL" not in _body(screen)  # but never the template line


# ---------------------------------------------------------------------------
# _compose_deps: Dependencies header, prefilled value, uv/npm placeholders,
# and the Python-constraint field (uv only)
# ---------------------------------------------------------------------------


async def test_compose_deps_python_header_value_and_python_field(tmp_path):
    """A uv-flavor (python) entry: the Dependencies header text + class, the comma-joined
    prefilled deps value, the uv placeholder, and the Python-constraint field's value +
    placeholder."""
    entry = store.add_python(_py(tmp_path, "print(1)\n"), name="dep")
    entry = store.update_dependencies(entry.slug, ["rich", "click"], requires_python=">=3.11")
    async with tui.MenuApp().run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        pilot.app.push_screen(screen)
        await pilot.pause()

        deps_input = screen.query_one("#st-deps", Input)
        header = _prev_sibling(deps_input)
        assert isinstance(header, Static)
        assert str(header.render()).strip() == "Dependencies"  # kills text drop / XX / case
        assert header.has_class("section")  # kills classes None / drop / XXsectionXX / SECTION

        # value=", ".join(deps) — kills value=None, dropped value, and the "XX, XX" separator.
        assert deps_input.value == "rich, click"
        # uv placeholder (kills dropped ternary + XX/upper text + the `and`/`is None`/`!= uv`
        # /`== XXuvXX` /`== UV` guard mutants that would flip it to the npm copy).
        assert deps_input.placeholder == "comma separated, e.g. requests>=2,<3, rich"

        python_input = screen.query_one("#st-python", Input)
        assert python_input.value == ">=3.11"  # kills value=None / dropped value
        assert (
            python_input.placeholder == 'Python constraint, e.g. ">=3.11" (empty = automatic)'
        )  # kills placeholder=None / dropped / gettext(None) / XX / lower / upper


async def test_compose_deps_js_uses_the_npm_placeholder(tmp_path):
    """An npm-flavor (js) copy entry takes the ELSE branch of the placeholder ternary — the
    npm example, not the uv one — and has no Python-constraint field."""
    src = tmp_path / "a.mjs"
    src.write_text("console.log(1);\n", encoding="utf-8")
    entry = store.add_script(src, kind="js", name="jsx")
    async with tui.MenuApp().run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        pilot.app.push_screen(screen)
        await pilot.pause()
        deps_input = screen.query_one("#st-deps", Input)
        # kills the `and`->`or` guard mutant (which would show the uv copy) and the XX/upper
        # mutants on the npm example string.
        assert deps_input.placeholder == "comma separated, e.g. chalk@^5, zod"
        assert not screen.query("#st-python")  # the Python field is uv-only


# ---------------------------------------------------------------------------
# _compose_needs: header, prefilled value, placeholder
# ---------------------------------------------------------------------------


async def test_compose_needs_header_value_and_placeholder(tmp_path):
    """Every kind gets the Needs section: header text + class, the comma-joined prefilled
    value, and the placeholder copy."""
    sh = tmp_path / "d.sh"
    sh.write_text("#!/bin/bash\necho hi\n", encoding="utf-8")
    entry = store.add_script(sh, kind="shell", name="d")
    entry = store.update_needs(entry.slug, ["ffmpeg", "jq"])
    async with tui.MenuApp().run_test() as pilot:
        screen = ScriptSettingsScreen(entry)
        pilot.app.push_screen(screen)
        await pilot.pause()

        needs_input = screen.query_one("#st-needs", Input)
        header = _prev_sibling(needs_input)
        assert isinstance(header, Static)
        assert str(header.render()).strip() == "Needs (external commands)"  # kills XX text
        assert header.has_class("section")  # kills classes None / drop / XXsectionXX / SECTION

        assert needs_input.value == "ffmpeg, jq"  # kills the "XX, XX" separator
        assert needs_input.placeholder == "comma separated, e.g. ffmpeg, jq"  # kills drop/XX/upper
