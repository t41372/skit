"""Exact-behavior coverage for the Preferences (,) and Health check (D) screens.

Both screens are pure presentation glue over config / store / launcher. These tests
assert the OBSERVABLE contracts of the branches those two modules own: what each screen
renders for a given saved state (editor fallback hint, pre-selected mirror radio, the
"issues" list, mirror-on line, the uv-missing warning), and what its actions mutate or
dismiss with (the save validator's https guard, disable/preset/custom persistence, the
jump-to-script slug, the rebuild report). Nothing is executed for its own sake.
"""

from __future__ import annotations

import pytest
from textual.widgets import Input, OptionList, RadioButton, RadioSet, Select, Static

from skit import config, launcher, metawriter, store, tui
from skit.metawriter import ParamSpec
from skit.paths import scripts_dir
from skit.tui_health import HealthScreen
from skit.tui_prefs import PreferencesScreen


@pytest.fixture(autouse=True)
def tmp_store(tmp_path, monkeypatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _py(tmp_path, body: str, name: str = "job.py"):
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _screen_text(screen) -> str:
    """Every Static's plain render, joined — the visible chrome of a composed screen."""
    return "\n".join(str(w.render()) for w in screen.query(Static))


# The metawriter recipe from test_tui_mut's drift test: GONE is declared as a managed
# param but never assigned in the body, so plan_for_entry reports a drift line for it.
_DRIFTED = metawriter.write_params(
    "CITY = 'x'\nprint(CITY)\n",
    [
        ParamSpec(name="CITY", kind="const", type="str"),
        ParamSpec(name="GONE", kind="const", type="str"),
    ],
)


def _rich_health_setup(tmp_path) -> str:
    """A command entry (skipped by the python-only drift scan) + a drifted python entry
    (the one issue) + an enabled mirror. Returns the drifted entry's slug."""
    store.add_command("echo hi", name="cmd")
    drifted = store.add_python(_py(tmp_path, _DRIFTED, "drifty.py"), name="drifty")
    config.save_mirror(config.preset("tsinghua"))
    return drifted.slug


# ---------------------------------------------------------------------------
# PreferencesScreen — compose branches
# ---------------------------------------------------------------------------


async def test_prefs_editor_fallback_hint_names_visual_editor(tmp_path, monkeypatch):
    """With $VISUAL set, the editor field advertises what "empty" resolves to."""
    monkeypatch.setenv("VISUAL", "vim")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        text = _screen_text(app.screen)
        assert "Empty means: vim (from $VISUAL / $EDITOR)" in text


async def test_prefs_enabled_preset_preselects_its_radio(tmp_path):
    """A saved preset mirror pre-selects the matching radio (not "custom"/"off"), and the
    custom URL inputs stay hidden because the selection isn't custom."""
    config.save_mirror(config.preset("tsinghua"))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        rs = app.screen.query_one("#pf-mirror", RadioSet)
        assert rs.pressed_index == 0  # "tsinghua" is _MIRROR_CHOICES[0]
        button = rs.pressed_button
        assert button is not None
        assert str(button.label) == "tsinghua"
        assert app.screen.query_one("#pf-pypi", Input).display is False


async def test_prefs_enabled_custom_url_preselects_custom(tmp_path):
    """An enabled mirror whose pypi matches no preset resolves to "custom", and the custom
    URL inputs are revealed and prefilled with the saved values."""
    config.save_mirror(
        config.MirrorConfig(
            enabled=True,
            pypi="https://corp.internal/simple",
            python_install="https://corp.internal/py/",
            uv_binary="https://corp.internal/uv/",
        )
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        rs = app.screen.query_one("#pf-mirror", RadioSet)
        assert rs.pressed_index == 3  # "custom"
        pypi = app.screen.query_one("#pf-pypi", Input)
        assert pypi.display is True
        assert pypi.value == "https://corp.internal/simple"


async def test_prefs_selecting_custom_reveals_url_inputs(tmp_path):
    """Switching the mirror radio to "custom" fires the change handler, which unhides the
    three URL inputs live (they start hidden under the default "off")."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        pypi = app.screen.query_one("#pf-pypi", Input)
        assert pypi.display is False  # off → hidden
        buttons = list(app.screen.query_one("#pf-mirror", RadioSet).query(RadioButton))
        buttons[3].value = True  # click "custom"
        await pilot.pause()
        assert pypi.display is True
        assert app.screen.query_one("#pf-uv-error", Static).display is True


# ---------------------------------------------------------------------------
# PreferencesScreen — action_save / action_close
# ---------------------------------------------------------------------------


async def test_prefs_save_off_persists_editor_form_and_disables_mirror(tmp_path):
    """Saving with the default "off" mirror writes editor + form, clears the language
    (still "auto"), disables the mirror, and dismisses True."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        screen.query_one("#pf-editor", Input).value = "micro"
        list(screen.query_one("#pf-form", RadioSet).query(RadioButton))[1].value = True  # plain
        list(screen.query_one("#pf-after", RadioSet).query(RadioButton))[1].value = True  # stay
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert config.load_editor() == "micro"
    assert config.load_form() == "plain"
    assert config.load_after_run() == "stay"
    assert config.load_mirror().enabled is False
    assert config.load_config().get("language") is None  # "auto" clears the key
    assert results == [True]


async def test_prefs_save_custom_non_https_uv_is_blocked(tmp_path):
    """The uv binary is downloaded and executed, so a non-https uv mirror is refused: the
    save shows the https error, saves no mirror, and does not dismiss."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        list(screen.query_one("#pf-mirror", RadioSet).query(RadioButton))[3].value = True  # custom
        await pilot.pause()
        screen.query_one("#pf-uv", Input).value = "http://mirror.example/uv"
        screen.action_save()
        await pilot.pause()
        error = str(app.screen.query_one("#pf-uv-error", Static).render())
    assert "https://" in error
    assert "http://mirror.example/uv" in error
    assert results == []  # blocked, not dismissed
    assert config.load_mirror().enabled is False  # nothing persisted for the mirror


async def test_prefs_save_custom_https_urls_persist(tmp_path):
    """A valid custom mirror (https uv) persists all three URLs and dismisses True."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        list(screen.query_one("#pf-mirror", RadioSet).query(RadioButton))[3].value = True  # custom
        await pilot.pause()
        screen.query_one("#pf-pypi", Input).value = "https://my.index/simple"
        screen.query_one("#pf-pyinstall", Input).value = "https://my.pyinstall/"
        screen.query_one("#pf-uv", Input).value = "https://my.uv/"
        screen.action_save()
        await pilot.pause()
    mirror = config.load_mirror()
    assert mirror.enabled is True
    assert mirror.pypi == "https://my.index/simple"
    assert mirror.python_install == "https://my.pyinstall/"
    assert mirror.uv_binary == "https://my.uv/"
    assert results == [True]


async def test_prefs_save_preset_and_explicit_language(tmp_path):
    """Selecting a preset radio saves that preset verbatim, and a non-"auto" language
    selection is written to config."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        next(
            iter(screen.query_one("#pf-mirror", RadioSet).query(RadioButton))
        ).value = True  # tsinghua
        screen.query_one("#pf-lang", Select).value = "en"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert config.load_mirror() == config.preset("tsinghua")
    assert config.load_config().get("language") == "en"
    assert results == [True]


async def test_prefs_close_dismisses_false(tmp_path):
    """Back (Esc) leaves everything untouched and dismisses False."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        screen.action_close()
        await pilot.pause()
    assert results == [False]


# ---------------------------------------------------------------------------
# HealthScreen — compose branches
# ---------------------------------------------------------------------------


async def test_health_uv_missing_shows_install_hint(tmp_path, monkeypatch):
    """When uv can't be found, the checklist shows the install pointer, not a path."""
    monkeypatch.setattr(launcher, "find_uv", lambda: None)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen())
        await pilot.pause()
        text = _screen_text(app.screen)
    assert "uv: not found" in text
    assert "docs.astral.sh/uv" in text


async def test_health_lists_drift_issue_and_mirror_on(tmp_path):
    """A drifted python entry becomes one selectable issue (command entries are skipped),
    and an enabled mirror renders its "on" line with the index URL."""
    slug = _rich_health_setup(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen())
        await pilot.pause()
        text = _screen_text(app.screen)
        assert "Issues (Enter jumps to the script):" in text
        assert "Mirror: on" in text
        assert "pypi.tuna.tsinghua.edu.cn" in text
        issues = app.screen.query_one("#hc-issues", OptionList)
        assert issues.option_count == 1  # only the drifted python entry, not the command
        option = issues.get_option_at_index(0)
        assert option.id == slug
        assert "out of sync" in str(option.prompt)


# ---------------------------------------------------------------------------
# HealthScreen — jump / rebuild / close actions
# ---------------------------------------------------------------------------


async def test_health_selecting_issue_jumps_to_that_script(tmp_path):
    """Selecting an issue row dismisses the screen with that script's slug (the Library
    then jumps to it)."""
    slug = _rich_health_setup(tmp_path)
    results: list[str | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen(), results.append)
        await pilot.pause()
        issues = app.screen.query_one("#hc-issues", OptionList)
        issues.highlighted = 0
        issues.action_select()
        await pilot.pause()
    assert results == [slug]


async def test_health_action_jump_dismisses_to_highlighted(tmp_path):
    """The Enter/footer twin dismisses to the highlighted issue's slug."""
    slug = _rich_health_setup(tmp_path)
    results: list[str | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen(), results.append)
        await pilot.pause()
        app.screen.query_one("#hc-issues", OptionList).highlighted = 0
        assert isinstance(app.screen, HealthScreen)
        app.screen.action_jump()
        await pilot.pause()
    assert results == [slug]


async def test_health_action_jump_noop_when_nothing_highlighted(tmp_path):
    """Issues exist but no row is highlighted (highlighted is None): action_jump has no
    target and must not dismiss — the guard's other arm."""
    _rich_health_setup(tmp_path)
    results: list[str | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen(), results.append)
        await pilot.pause()
        issues = app.screen.query_one("#hc-issues", OptionList)
        issues.highlighted = None
        assert isinstance(app.screen, HealthScreen)
        app.screen.action_jump()
        await pilot.pause()
        assert isinstance(app.screen, HealthScreen)  # not dismissed
    assert results == []


async def test_health_action_jump_is_noop_when_healthy(tmp_path):
    """With no issue list to jump into, action_jump does nothing (no dismiss)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="ok")
    results: list[str | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen(), results.append)
        await pilot.pause()
        assert not app.screen.query("#hc-issues")  # nothing to jump to
        assert isinstance(app.screen, HealthScreen)
        app.screen.action_jump()
        await pilot.pause()
        assert isinstance(app.screen, HealthScreen)  # still here
    assert results == []


async def test_health_rebuild_reports_count_and_problems(tmp_path):
    """R rebuilds the registry in place: the report names the rebuilt count and any
    problem directories (a stray dir with no meta.toml is skipped and reported)."""
    store.add_python(_py(tmp_path, "print(1)\n"), name="ok")
    (scripts_dir() / "rogue").mkdir(parents=True, exist_ok=True)  # meta-less stray dir
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen())
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, HealthScreen)
        screen.action_rebuild()
        await pilot.pause()
        report = str(screen.query_one("#hc-rebuilt", Static).render())
    assert "Index rebuilt: 1 entry" in report
    assert "rogue: meta.toml is missing; skipped" in report


async def test_health_close_dismisses_none(tmp_path):
    """Back (Esc) dismisses with None — the Library selection is left unchanged."""
    results: list[str | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, HealthScreen)
        screen.action_close()
        await pilot.pause()
    assert results == [None]
