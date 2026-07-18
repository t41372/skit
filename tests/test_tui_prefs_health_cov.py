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

from conftest import full_mirror
from skit import config, store, tui
from skit.langs.python import metawriter
from skit.params import ParamDecl
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
        ParamDecl(name="CITY", binding="const", type="str"),
        ParamDecl(name="GONE", binding="const", type="str"),
    ],
)


def _rich_health_setup(tmp_path) -> str:
    """A command entry (skipped by the python-only drift scan) + a drifted python entry
    (the one issue) + an enabled mirror. Returns the drifted entry's slug."""
    store.add_command("echo hi", name="cmd")
    drifted = store.add_python(_py(tmp_path, _DRIFTED, "drifty.py"), name="drifty")
    config.save_mirror(full_mirror())
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


async def test_prefs_enabled_presets_preselect_each_axis_radio(tmp_path):
    """A saved full mirror pre-selects the matching preset radio on EACH axis row (never
    "custom"/"off"), and every custom URL input stays hidden."""
    config.save_mirror(full_mirror())
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        for row, label in (
            ("#pf-mirror-pypi", "tsinghua"),
            ("#pf-mirror-github", "nju"),
            ("#pf-mirror-npm", "npmmirror"),
        ):
            rs = app.screen.query_one(row, RadioSet)
            assert rs.pressed_index == 0, row  # each axis's preset is its choice list's head
            button = rs.pressed_button
            assert button is not None
            assert str(button.label) == label
        for wid in ("#pf-pypi", "#pf-github", "#pf-npm"):
            assert app.screen.query_one(wid, Input).display is False, wid


async def test_prefs_enabled_custom_url_preselects_custom_only_on_its_axis(tmp_path):
    """Axes resolve independently: custom pypi/github URLs select "custom" on those rows
    and reveal their inputs, while the untouched npm axis stays "off" and hidden."""
    config.save_mirror(
        config.compose(
            pypi="https://corp.internal/simple",
            python_install="https://corp.internal/py/",
            uv_binary="https://corp.internal/uv/",
        )
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        rs = app.screen.query_one("#pf-mirror-pypi", RadioSet)
        assert rs.pressed_index == len(config.PYPI_PRESETS)  # "custom"
        pypi = app.screen.query_one("#pf-pypi", Input)
        assert pypi.display is True
        assert pypi.value == "https://corp.internal/simple"
        assert app.screen.query_one("#pf-mirror-github", RadioSet).pressed_index == len(
            config.GITHUB_RELEASE_PRESETS
        )
        assert app.screen.query_one("#pf-github", Input).display is True
        npm_rs = app.screen.query_one("#pf-mirror-npm", RadioSet)
        assert npm_rs.pressed_index == len(config.NPM_PRESETS) + 1  # "off"
        assert app.screen.query_one("#pf-npm", Input).display is False


async def test_prefs_selecting_custom_reveals_only_that_axis_inputs(tmp_path):
    """Switching ONE axis to "custom" unhides only that axis's URL input; the other axes'
    inputs stay hidden until their own rows go custom. The single error slot is shared and
    appears as soon as any axis is custom."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen())
        await pilot.pause()
        pypi = app.screen.query_one("#pf-pypi", Input)
        github = app.screen.query_one("#pf-github", Input)
        npm = app.screen.query_one("#pf-npm", Input)
        error = app.screen.query_one("#pf-mirror-error", Static)
        assert pypi.display is False  # off → hidden
        assert error.display is False  # nothing custom yet → the error slot is hidden
        buttons = list(app.screen.query_one("#pf-mirror-pypi", RadioSet).query(RadioButton))
        buttons[len(config.PYPI_PRESETS)].value = True  # click pypi "custom"
        await pilot.pause()
        assert pypi.display is True
        # Only the pypi input revealed; the github/npm inputs stay hidden under their own rows.
        assert github.display is False
        assert npm.display is False
        # The shared error slot appears as soon as ANY axis is custom.
        assert error.display is True
        gh_buttons = list(app.screen.query_one("#pf-mirror-github", RadioSet).query(RadioButton))
        gh_buttons[len(config.GITHUB_RELEASE_PRESETS)].value = True  # github "custom"
        await pilot.pause()
        assert github.display is True


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
    """The github base derives the uv binary (downloaded and executed), so a non-https base is
    refused: the save shows the https error, saves no mirror, and does not dismiss."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        gh = list(screen.query_one("#pf-mirror-github", RadioSet).query(RadioButton))
        gh[len(config.GITHUB_RELEASE_PRESETS)].value = True  # github "custom"
        await pilot.pause()
        screen.query_one("#pf-github", Input).value = "http://mirror.example/gh"
        screen.action_save()
        await pilot.pause()
        error = str(app.screen.query_one("#pf-mirror-error", Static).render())
    # Letter-exact: the security rationale is the message — a mangled version must fail.
    assert (
        "The uv binary is downloaded and executed, so the github-release base URL must "
        "use https:// (got: http://mirror.example/gh)." in error
    )
    assert "XX" not in error  # an XX-wrapped msgid contains the original as a substring
    assert results == []  # blocked, not dismissed
    assert config.load_mirror().enabled is False  # nothing persisted for the mirror


async def test_prefs_save_custom_https_urls_persist(tmp_path):
    """Custom pypi + github axes (https uv) persist their URLs and dismiss True — and the
    npm axis, left "off", stays empty rather than inheriting anyone's vendor."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        pypi_rb = list(screen.query_one("#pf-mirror-pypi", RadioSet).query(RadioButton))
        pypi_rb[len(config.PYPI_PRESETS)].value = True  # pypi "custom"
        gh_rb = list(screen.query_one("#pf-mirror-github", RadioSet).query(RadioButton))
        gh_rb[len(config.GITHUB_RELEASE_PRESETS)].value = True  # github "custom"
        await pilot.pause()
        screen.query_one("#pf-pypi", Input).value = "https://my.index/simple"
        screen.query_one("#pf-github", Input).value = "https://my.gh/"  # one base, both vectors
        screen.action_save()
        await pilot.pause()
    mirror = config.load_mirror()
    assert mirror.enabled is True
    assert mirror.pypi == "https://my.index/simple"
    # The single github base expands to both github-release vectors.
    assert mirror.python_install == "https://my.gh/astral-sh/python-build-standalone/"
    assert mirror.uv_binary == "https://my.gh/astral-sh/uv"
    assert mirror.npm == ""  # the off axis saved nothing
    assert results == [True]


async def test_prefs_save_presets_and_explicit_language(tmp_path):
    """Selecting each axis's preset radio saves that axis's URLs verbatim, and a
    non-"auto" language selection is written to config."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        for row in ("#pf-mirror-pypi", "#pf-mirror-github", "#pf-mirror-npm"):
            next(iter(screen.query_one(row, RadioSet).query(RadioButton))).value = True
        screen.query_one("#pf-lang", Select).value = "en"
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert config.load_mirror() == full_mirror()
    assert config.load_config().get("language") == "en"
    assert results == [True]


async def test_prefs_save_single_axis_preset_leaves_others_off(tmp_path):
    """The regression this design fix exists for, at the TUI level: picking a PyPI vendor
    configures the PyPI axis and NOTHING else."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        next(iter(screen.query_one("#pf-mirror-pypi", RadioSet).query(RadioButton))).value = True
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    m = config.load_mirror()
    assert m.enabled
    assert m.pypi == config.PYPI_PRESETS["tsinghua"]
    assert (m.python_install, m.uv_binary, m.npm) == ("", "", "")
    assert results == [True]


async def test_prefs_paused_config_language_only_save_keeps_mirror_intact(tmp_path):
    """F1: a paused config (custom pypi URL saved, then master off) opened in Preferences,
    with ONLY the language changed and saved, leaves the [mirror] block intact — the paused
    URL survives and enabled stays False (the master row is pre-selected "off")."""
    config.save_mirror(config.compose(pypi="https://corp.internal/simple"))
    config.disable()  # now paused: URL kept, enabled False
    before = config.load_mirror()
    assert not before.enabled
    assert before.pypi == "https://corp.internal/simple"
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        master = screen.query_one("#pf-mirror-master", RadioSet)
        assert master.pressed_button is not None
        assert str(master.pressed_button.label) == "off"  # paused reflects as master "off"
        screen.query_one("#pf-lang", Select).value = "en"  # change ONLY the language
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    after = config.load_mirror()
    assert after == before  # the whole [mirror] block is byte-for-byte untouched
    assert not after.enabled
    assert after.pypi == "https://corp.internal/simple"
    assert config.load_config().get("language") == "en"
    assert results == [True]


async def test_prefs_github_http_save_writes_nothing_not_even_language(tmp_path):
    """F7: a github custom base that isn't https:// is refused BEFORE any write, so the
    language and editor edits queued in the same save must not persist (no half-apply)."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        gh = list(screen.query_one("#pf-mirror-github", RadioSet).query(RadioButton))
        gh[len(config.GITHUB_RELEASE_PRESETS)].value = True  # github "custom"
        screen.query_one("#pf-editor", Input).value = "micro"
        screen.query_one("#pf-lang", Select).value = "en"
        await pilot.pause()
        screen.query_one("#pf-github", Input).value = "http://evil/gh"
        screen.action_save()
        await pilot.pause()
        assert isinstance(app.screen, PreferencesScreen)  # not dismissed
    assert results == []  # blocked
    assert config.load_editor() == ""  # editor NOT written
    assert config.load_config().get("language") is None  # language NOT written
    assert config.load_mirror().enabled is False


async def test_prefs_custom_left_empty_blocks_save_with_no_writes(tmp_path):
    """F8: a custom axis with an empty URL must not silently save as "off" (the radio would
    then lie). The save shows "A custom choice needs a URL." and writes nothing at all."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        npm = list(screen.query_one("#pf-mirror-npm", RadioSet).query(RadioButton))
        npm[len(config.NPM_PRESETS)].value = True  # npm "custom"
        screen.query_one("#pf-editor", Input).value = "micro"
        await pilot.pause()
        # #pf-npm is left empty on purpose.
        screen.action_save()
        await pilot.pause()
        error = str(app.screen.query_one("#pf-mirror-error", Static).render())
        assert isinstance(app.screen, PreferencesScreen)  # not dismissed
    assert "A custom choice needs a URL." in error
    assert "XX" not in error  # an XX-wrapped msgid contains the original as a substring
    assert results == []
    assert config.load_editor() == ""  # no half-apply
    assert config.load_mirror().enabled is False


@pytest.mark.parametrize("bad", ["tsinghua", "https://a b/simple"])
async def test_prefs_custom_non_url_value_blocks_save(tmp_path, bad):
    """R2-4: the TUI custom inputs share the CLI/wizard is_url_token gate — a vendor-name
    typo or display prose in a custom URL input blocks the save with the same inline error
    and writes nothing (it must not persist to surface later as a broken registry)."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        npm = list(screen.query_one("#pf-mirror-npm", RadioSet).query(RadioButton))
        npm[len(config.NPM_PRESETS)].value = True  # npm "custom"
        screen.query_one("#pf-editor", Input).value = "micro"
        await pilot.pause()
        screen.query_one("#pf-npm", Input).value = bad
        screen.action_save()
        await pilot.pause()
        error = str(app.screen.query_one("#pf-mirror-error", Static).render())
        assert isinstance(app.screen, PreferencesScreen)  # not dismissed
    assert "A custom choice needs a URL." in error
    assert "XX" not in error  # an XX-wrapped msgid contains the original as a substring
    assert results == []
    assert config.load_editor() == ""  # no half-apply
    assert config.load_mirror().npm == ""  # the typo never landed on disk


async def test_prefs_github_base_with_whitespace_blocks_save(tmp_path):
    """R3-1: the github base input shares the SAME is_url_token gate as the CLI and wizard
    — an https:// base with a space would pass the https check, persist, and blow up much
    later inside the uv bootstrap. It must be refused inline with nothing written."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        gh = list(screen.query_one("#pf-mirror-github", RadioSet).query(RadioButton))
        gh[len(config.GITHUB_RELEASE_PRESETS)].value = True  # github "custom"
        screen.query_one("#pf-editor", Input).value = "micro"
        await pilot.pause()
        screen.query_one("#pf-github", Input).value = "https://my mirror/gh"
        screen.action_save()
        await pilot.pause()
        error = str(app.screen.query_one("#pf-mirror-error", Static).render())
        assert isinstance(app.screen, PreferencesScreen)  # not dismissed
    assert "A custom choice needs a URL." in error
    assert "XX" not in error
    assert results == []
    assert config.load_editor() == ""  # no half-apply
    assert config.load_mirror().uv_binary == ""  # the garbage base never expanded to disk


async def test_prefs_underivable_github_pair_survives_a_language_only_save(tmp_path):
    """R2-1: a hand-edited github pair no base derives prefills the base input EMPTY (there
    is no base to show). An untouched save — e.g. changing only the language — must pass the
    stored pair through as-is instead of refusing the whole form over an axis the user never
    touched."""
    config.save_mirror(
        config.compose(python_install="https://weird/pbs/", uv_binary="https://other/uv")
    )
    before = config.load_mirror()
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        # The underivable pair reads "custom" with an empty base prefill.
        gh_rs = screen.query_one("#pf-mirror-github", RadioSet)
        assert gh_rs.pressed_index == len(config.GITHUB_RELEASE_PRESETS)  # "custom"
        assert screen.query_one("#pf-github", Input).value == ""
        screen.query_one("#pf-lang", Select).value = "en"  # change ONLY the language
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    after = config.load_mirror()
    assert after == before  # the pair passed through byte-for-byte, still enabled
    assert after.python_install == "https://weird/pbs/"
    assert after.uv_binary == "https://other/uv"
    assert config.load_config().get("language") == "en"  # and the save DID land
    assert results == [True]


async def test_prefs_half_set_github_pair_also_passes_through(tmp_path):
    """R2-1 (edge): a HALF-set hand-edited pair (only one github vector stored) is just as
    underivable — each half stands alone in the passthrough guard (an or, never an and), so
    a language-only save preserves it too."""
    config.save_mirror(config.compose(python_install="https://weird/pbs/"))
    before = config.load_mirror()
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        screen.query_one("#pf-lang", Select).value = "en"  # change ONLY the language
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    after = config.load_mirror()
    assert after == before  # the half-pair survived the save
    assert after.python_install == "https://weird/pbs/"
    assert after.uv_binary == ""
    assert results == [True]


async def test_prefs_derivable_base_cleared_still_blocks_save(tmp_path):
    """R2-1 (guard): the passthrough is ONLY for underivable pairs. When the stored pair
    derives from a base (the input prefills it), clearing that input is a real user action
    and must error out, not silently pass the old pair through."""
    python_install, uv_binary = config.github_release_urls("https://my.gh")
    config.save_mirror(config.compose(python_install=python_install, uv_binary=uv_binary))
    before = config.load_mirror()
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        base_input = screen.query_one("#pf-github", Input)
        assert base_input.value == "https://my.gh"  # derivable pair prefills its base
        screen.query_one("#pf-lang", Select).value = "en"
        await pilot.pause()
        base_input.value = ""  # the user explicitly clears the base
        screen.action_save()
        await pilot.pause()
        error = str(app.screen.query_one("#pf-mirror-error", Static).render())
        assert isinstance(app.screen, PreferencesScreen)  # not dismissed
    assert "A custom choice needs a URL." in error
    assert "XX" not in error  # an XX-wrapped msgid contains the original as a substring
    assert results == []
    assert config.load_mirror() == before  # disk untouched
    assert config.load_config().get("language") is None  # the language write was refused too


async def test_prefs_github_custom_empty_base_blocks_save(tmp_path):
    """The github axis's own empty-custom guard: custom selected with the base left empty
    hits 'A custom choice needs a URL.' too, and nothing is saved."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(PreferencesScreen(), results.append)
        await pilot.pause()
        screen = app.screen
        assert isinstance(screen, PreferencesScreen)
        gh = list(screen.query_one("#pf-mirror-github", RadioSet).query(RadioButton))
        gh[len(config.GITHUB_RELEASE_PRESETS)].value = True  # github "custom", base left empty
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        error = str(app.screen.query_one("#pf-mirror-error", Static).render())
        assert isinstance(app.screen, PreferencesScreen)  # not dismissed
    assert "A custom choice needs a URL." in error
    assert "XX" not in error  # an XX-wrapped msgid contains the original as a substring
    assert results == []
    assert config.load_mirror().enabled is False


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
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: None)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen())
        await pilot.pause()
        text = _screen_text(app.screen)
    assert "uv: not found" in text
    assert "docs.astral.sh/uv" in text


async def test_health_lists_drift_issue_and_mirror_on(tmp_path):
    """A drifted python entry becomes one selectable issue (command entries are skipped),
    and an enabled mirror renders the per-axis summary line."""
    slug = _rich_health_setup(tmp_path)
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen())
        await pilot.pause()
        text = _screen_text(app.screen)
        assert "Issues (Enter jumps to the script):" in text
        assert "Mirrors: pypi=tsinghua" in text
        assert "github=nju" in text
        assert "npm=npmmirror" in text
        issues = app.screen.query_one("#hc-issues", OptionList)
        assert issues.option_count == 1  # only the drifted python entry, not the command
        option = issues.get_option_at_index(0)
        assert option.id == slug
        assert "out of sync" in str(option.prompt)


async def test_health_lists_missing_needs_issue(tmp_path, monkeypatch):
    """A shell entry whose declared `needs` command is off PATH becomes a selectable
    issue row naming the missing tool (the same sweep doctor prints)."""
    sh = tmp_path / "d.sh"
    sh.write_text("#!/bin/bash\necho hi\n", encoding="utf-8")
    entry = store.add_script(sh, kind="shell", name="d")
    store.update_needs("d", ["ffmpeg"])
    monkeypatch.setattr("shutil.which", lambda _name: None)  # nothing on PATH
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        app.push_screen(HealthScreen())
        await pilot.pause()
        issues = app.screen.query_one("#hc-issues", OptionList)
        prompts = [str(issues.get_option_at_index(i).prompt) for i in range(issues.option_count)]
    assert any("ffmpeg" in p and entry.slug for p in prompts)
    assert any("missing external command" in p for p in prompts)


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
