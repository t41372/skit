"""Mutation-hardening tests for the Preferences (,) screen (skit.tui_prefs).

These pin the OBSERVABLE contracts the existing coverage left un-nailed: the panel's
English border title, the "nothing selected -> off" per-axis fallback, the exact string
the save writes for the *default* (else-branch) form/after choices, and the widget each
save actually reads (the after-run radio must be `#pf-after`, not merely the first
RadioSet on screen). Every footer chip/key the screen advertises also gets a positive
pilot test. The verbatim https error copy and the per-axis custom reveal are already
letter-exact in test_tui_prefs_health_cov.py and are not repeated here. English catalog
throughout, so the message assertions read against the original msgids.
"""

from __future__ import annotations

import pytest
from textual.widgets import Input, OptionList, RadioButton, RadioSet, Static

from conftest import click_label
from skit import config, i18n, tui
from skit.tui_prefs import _MASTER_CHOICES, PreferencesScreen


@pytest.fixture(autouse=True)
def _en(monkeypatch):
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.set_language("en")


async def _open_prefs(app, on_result=None):
    """Push the Preferences screen and settle it; return the live screen."""
    if on_result is None:
        app.push_screen(PreferencesScreen())
    else:
        app.push_screen(PreferencesScreen(), on_result)
    return app.screen


# ---------------------------------------------------------------------------
# on_mount: the panel border title
# ---------------------------------------------------------------------------


async def test_on_mount_sets_english_border_title(tmp_path):
    """on_mount writes the translated panel title onto the body border. The English
    catalog resolves it to exactly "Preferences" — pins the msgid, the gettext call,
    and that a title is set at all (None/blank/case mutants all diverge)."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert str(screen.query_one("#pf-body").border_title) == "Preferences"


# ---------------------------------------------------------------------------
# _axis_choice: the "nothing pressed -> off" fallback
# ---------------------------------------------------------------------------


async def test_axis_choice_falls_back_to_off_when_no_button_pressed(tmp_path):
    """RadioSet.pressed_index is -1 when no button is pressed (its documented "none
    selected" state), which is outside the choices range, so _axis_choice must return
    the safe "off". The master row of a live preset config is deselected to reach that
    state; saving from it then takes the master-off branch — the mirror pauses (enabled
    False) while the stored pypi URL survives, exactly as if "off" had been pressed."""
    config.save_mirror(config.compose(pypi=config.PYPI_PRESETS["tsinghua"]))
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app, results.append)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        master_set = screen.query_one("#pf-mirror-master", RadioSet)
        assert master_set.pressed_index == 0  # an enabled config starts on "on"
        master_set._pressed_button = None  # documented "none pressed" state -> index -1
        assert master_set.pressed_index == -1
        assert screen._axis_choice("#pf-mirror-master", _MASTER_CHOICES) == "off"
        screen.action_save()
        await pilot.pause()
    mirror = config.load_mirror()
    assert mirror.enabled is False  # the "off" fallback paused the mirror
    assert mirror.pypi == config.PYPI_PRESETS["tsinghua"]  # pause, don't destroy
    assert results == [True]


# ---------------------------------------------------------------------------
# _toggle_custom: the shared error slot reveals for ANY single custom axis
# ---------------------------------------------------------------------------


async def test_error_slot_appears_for_a_lone_github_custom(tmp_path):
    """The error slot's display is an or-chain over the three axes. The cov suite's
    pypi-first flow never shows the slot for a lone github custom, which leaves the
    `github and npm` collapse of the chain alive — this pins the middle axis alone."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        error_slot = screen.query_one("#pf-mirror-error", Static)
        assert error_slot.display is False  # nothing custom yet
        gh = list(screen.query_one("#pf-mirror-github", RadioSet).query(RadioButton))
        gh[len(config.GITHUB_RELEASE_PRESETS)].value = True  # github "custom", alone
        await pilot.pause()
        assert error_slot.display is True


# ---------------------------------------------------------------------------
# action_save: the default (else-branch) form / after-run literals
# ---------------------------------------------------------------------------


async def test_save_persists_default_tui_form_and_exit_after_run(tmp_path):
    """A fresh screen has the first radio of each pair pressed (index 0), so save takes
    the *else* branch of both ternaries: form -> "tui", after-run -> "exit". Asserted
    against the RAW config file, because load_form()/load_after_run() normalise unknown
    values back to those same defaults and would mask a corrupted literal."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        assert screen.query_one("#pf-form", RadioSet).pressed_index == 0
        assert screen.query_one("#pf-after", RadioSet).pressed_index == 0
        screen.action_save()
        await pilot.pause()
    raw = config.load_config()
    assert raw["form"] == "tui"
    assert raw["after_run"] == "exit"


async def test_save_reads_after_run_from_pf_after_radioset(tmp_path):
    """The after-run save must read `#pf-after`, not the first RadioSet on the screen
    (that is `#pf-form`). Set the two to different indices — after=stay(1), form=tui(0) —
    so a query that fell back to the first RadioSet would persist "exit" instead of the
    "stay" the user actually chose."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        list(screen.query_one("#pf-after", RadioSet).query(RadioButton))[1].value = True  # stay
        await pilot.pause()
        assert screen.query_one("#pf-form", RadioSet).pressed_index == 0  # differs from after
        assert screen.query_one("#pf-after", RadioSet).pressed_index == 1
        screen.action_save()
        await pilot.pause()
    assert config.load_after_run() == "stay"
    assert config.load_config()["form"] == "tui"  # form still read from its own radio


# ---------------------------------------------------------------------------
# footer chips / key bindings — every advertised path is operable
# ---------------------------------------------------------------------------


async def test_footer_save_chip_saves_and_dismisses(tmp_path):
    """The "Ctrl+S Save" footer chip is a button: clicking it fires screen.save, which
    persists and dismisses True (mouse-only operability of the advertised action)."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test(size=(130, 30)) as pilot:
        screen = await _open_prefs(app, results.append)
        await pilot.pause()
        screen.query_one("#pf-editor", Input).value = "micro"
        await pilot.pause()
        await click_label(pilot, "#pf-keys", "Save")
    assert config.load_editor() == "micro"
    assert results == [True]


async def test_footer_back_chip_dismisses_false(tmp_path):
    """The "Esc Back" footer chip fires screen.close, dismissing False without saving."""
    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test(size=(130, 30)) as pilot:
        await _open_prefs(app, results.append)
        await pilot.pause()
        await click_label(pilot, "#pf-keys", "Back")
    assert results == [False]


async def test_ctrl_s_key_saves_and_escape_closes(tmp_path):
    """Keyboard twins of the two chips: Ctrl+S (priority binding — the key grammar's
    save chord) saves and dismisses True; on a fresh screen Esc closes and dismisses
    False."""
    saved: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app, saved.append)
        await pilot.pause()
        screen.query_one("#pf-editor", Input).value = "nvim"
        await pilot.pause()
        await pilot.press("ctrl+s")
        await pilot.pause()
    assert config.load_editor() == "nvim"
    assert saved == [True]

    closed: list[bool | None] = []
    app2 = tui.MenuApp()
    async with app2.run_test() as pilot:
        await _open_prefs(app2, closed.append)
        await pilot.pause()
        await pilot.press("escape")
        await pilot.pause()
    assert closed == [False]


# ---------------------------------------------------------------------------
# __init__: both edit-tracking flags start as the bool False (never None)
# ---------------------------------------------------------------------------


def test_init_dirty_flags_start_as_false():
    """__init__ seeds `_dirty` and `_dirt_armed` to the bool False. `_dirty` gates the Esc
    discard guard (`not self._dirty`) and `_dirt_armed` gates the Changed handler until the
    mount settles — a stray None would read as falsy but is the wrong type. Constructed
    directly (no app) so the assertion sees the __init__ value before on_mount arms it."""
    screen = PreferencesScreen()
    assert screen._dirty is False
    assert screen._dirt_armed is False


# ---------------------------------------------------------------------------
# _refresh_runner_count: singular / plural / comma-join / empty, letter-exact
# ---------------------------------------------------------------------------


def _runner(name: str) -> config.PromptRunner:
    return config.PromptRunner(name=name, argv=(name, "{{prompt}}"))


async def test_runner_count_singular_is_letter_exact(tmp_path):
    """Exactly one configured agent takes ngettext's SINGULAR arm and names it. Letter-exact
    against the English msgid — the count value, the singular wording, and the name — so a
    None/XX-wrapped/upper-cased msgid all diverge (the plural cov tests never hit n == 1)."""
    config.save_prompt_runners([_runner("solo")])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        line = str(screen.query_one("#pf-runner-count", Static).render())
    assert line == "1 agent configured: solo"


async def test_runner_count_plural_joins_names_with_comma_space(tmp_path):
    """Two agents take the PLURAL arm and their names join with exactly ", ". Letter-exact,
    so an XX-wrapped plural msgid or a mangled "XX, XX" separator diverges (the cov suite
    only checks a substring of the plural line, which an XX-wrap still satisfies)."""
    config.save_prompt_runners([_runner("alpha"), _runner("beta")])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        line = str(screen.query_one("#pf-runner-count", Static).render())
    assert line == "2 agents configured: alpha, beta"


async def test_runner_count_empty_is_letter_exact(tmp_path, monkeypatch):
    """No agents: the else-branch gettext renders letter-exact "No agents configured." (the
    cov suite checks only a substring, which an XX-wrapped msgid would still satisfy)."""
    monkeypatch.setattr(config, "ensure_prompt_runners_seeded", lambda: None)
    config.save_prompt_runners([])
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        line = str(screen.query_one("#pf-runner-count", Static).render())
    assert line == "No agents configured."


# ---------------------------------------------------------------------------
# _compose_mirror: the master switch pressed-state across the three stored states
# ---------------------------------------------------------------------------


async def test_master_radio_fresh_config_presses_on(tmp_path):
    """A fresh config (nothing stored, nothing to pause) reads master = "on": the row is
    exactly the two buttons "on"/"off" and "on" (index 0) is pressed. Pins the whole
    RadioButton value mapping `(choice == "on") == master_on` — a None/inverted/dropped
    value, a mutated "on" literal, or a dropped label all diverge from this."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        master = screen.query_one("#pf-mirror-master", RadioSet)
        assert [str(b.label) for b in master.query(RadioButton)] == ["on", "off"]
        assert master.pressed_index == 0
        assert master.pressed_button is not None
        assert str(master.pressed_button.label) == "on"


async def test_master_radio_paused_pypi_only_presses_off(tmp_path):
    """A paused config with a stored URL reads master = "off" (pause, don't destroy). A lone
    pypi URL exercises the pypi term of the `any-axis-set` test — the discriminating case a
    fresh config can't reach, since the master row auto-defaults to index 0 when unpressed."""
    config.save_mirror(config.MirrorConfig(enabled=False, pypi="https://corp.internal/simple"))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        master = screen.query_one("#pf-mirror-master", RadioSet)
        assert master.pressed_index == 1
        assert master.pressed_button is not None
        assert str(master.pressed_button.label) == "off"


async def test_master_radio_paused_uv_binary_only_presses_off(tmp_path):
    """The same paused-master contract driven by a lone uv_binary URL — the github term of
    the any-axis-set test. This kills the or-chain associativity mutants a pypi-only paused
    config leaves alive (the middle/right operands only matter when the left ones are empty)."""
    config.save_mirror(config.MirrorConfig(enabled=False, uv_binary="https://corp.internal/uv"))
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        master = screen.query_one("#pf-mirror-master", RadioSet)
        assert master.pressed_index == 1
        assert master.pressed_button is not None
        assert str(master.pressed_button.label) == "off"


# ---------------------------------------------------------------------------
# _compose_mirror: one walk over the whole section — strings, ids, classes, inputs
# ---------------------------------------------------------------------------


async def test_mirror_section_is_letter_exact_end_to_end(tmp_path):
    """Walk the composed three-axis mirror section against a config that puts every axis on
    a custom URL, and pin — letter-exact, English catalog — every visible label, every
    widget id, every `classes=` value, every input value/placeholder, and each axis's
    pressed radio. One assertion wall for the section's XX-wrap / case / classes=None /
    dropped-argument mutants at once."""
    py, uv = config.github_release_urls("https://g.example")
    config.save_mirror(
        config.compose(
            pypi="https://p.example/simple",
            python_install=py,
            uv_binary=uv,
            npm="https://n.example",
        )
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)

        # Section header + the per-axis-independence hint (both keep their own class).
        section_texts = [str(w.render()) for w in screen.query("Static.section")]
        assert "Download mirrors (mainland-China acceleration)" in section_texts
        hint_texts = [str(w.render()) for w in screen.query("Static.hint")]
        assert "Each ecosystem is its own choice — mirror vendors differ per axis." in hint_texts

        # The four "pf-axis" labels, in order — text and class together (a dropped/mutated
        # class drops the widget from this query, a mutated msgid mismatches the text).
        assert [str(w.render()) for w in screen.query("Static.pf-axis")] == [
            'Master switch — "off" pauses mirrors but keeps the saved URLs.',
            "PyPI index (Python packages)",
            "GitHub releases (Python builds, the uv binary)",
            "npm registry (JS/TS packages)",
        ]

        # The four side-by-side radio rows: id + "pf-mirror-row" class, in document order.
        rows = list(screen.query("RadioSet.pf-mirror-row"))
        assert [rs.id for rs in rows] == [
            "pf-mirror-master",
            "pf-mirror-pypi",
            "pf-mirror-github",
            "pf-mirror-npm",
        ]

        def labels(row_id: str) -> list[str]:
            rs = screen.query_one(row_id, RadioSet)
            return [str(b.label) for b in rs.query(RadioButton)]

        # Each row's button labels are its choice vocabulary, verbatim (dropped-label mutants).
        assert labels("#pf-mirror-master") == ["on", "off"]
        assert labels("#pf-mirror-pypi") == ["tsinghua", "aliyun", "ustc", "custom", "off"]
        assert labels("#pf-mirror-github") == ["nju", "custom", "off"]
        assert labels("#pf-mirror-npm") == ["npmmirror", "custom", "off"]

        def pressed_label(row_id: str) -> str:
            button = screen.query_one(row_id, RadioSet).pressed_button
            assert button is not None, row_id
            return str(button.label)

        # A custom URL on each axis selects that axis's "custom" radio (value mapping).
        assert pressed_label("#pf-mirror-pypi") == "custom"
        assert pressed_label("#pf-mirror-github") == "custom"
        assert pressed_label("#pf-mirror-npm") == "custom"

        # Each axis input carries its stored value and its own placeholder msgid.
        pypi_input = screen.query_one("#pf-pypi", Input)
        assert pypi_input.value == "https://p.example/simple"
        assert str(pypi_input.placeholder) == "PyPI index URL"
        github_input = screen.query_one("#pf-github", Input)
        assert github_input.value == "https://g.example"
        assert str(github_input.placeholder) == "github-release mirror base URL"
        npm_input = screen.query_one("#pf-npm", Input)
        assert npm_input.value == "https://n.example"
        assert str(npm_input.placeholder) == "npm registry URL"

        # The shared error slot: its own "error" class, empty until a save fails.
        error_slot = screen.query_one("#pf-mirror-error", Static)
        assert "error" in error_slot.classes
        assert str(error_slot.render()) == ""


# ---------------------------------------------------------------------------
# action_save: the js-runtime persistence boundary (auto -> "" ; first runner)
# ---------------------------------------------------------------------------


async def test_save_js_auto_clears_the_stored_runner_key(tmp_path):
    """Selecting Automatic (radio index 0, `js_index <= 0`) saves the empty literal, which
    clears the js.runner key entirely. Asserted against the RAW config because load_js_runner
    normalizes any unknown value (an "XXXX" mutant) back to "" and would mask it."""
    config.save_js_runner("bun")
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        js = screen.query_one("#pf-js", RadioSet)
        assert js.pressed_index == 2  # the saved bun is preselected
        next(iter(js.query(RadioButton))).value = True  # Automatic (index 0)
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert config.load_config().get("js", {}).get("runner") is None


async def test_save_js_first_runner_persists_that_runner(tmp_path):
    """Selecting the FIRST js runtime (deno, radio index 1) persists "deno". The `<= 0`
    boundary: index 1 must fall to the else arm `JS_RUNNERS[js_index - 1]` (deno), never back
    to empty — a `<= 1` mutant would silently drop the user's first-runtime choice."""
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        list(screen.query_one("#pf-js", RadioSet).query(RadioButton))[1].value = True  # deno
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
    assert config.load_js_runner() == "deno"


# ---------------------------------------------------------------------------
# action_save: the Windows bash-path "No such file" copy, letter-exact
# ---------------------------------------------------------------------------


async def test_save_bash_missing_file_error_is_letter_exact(tmp_path, monkeypatch):
    """On Windows a bash path that is not a file blocks the save with letter-exact
    "No such file: <path>" (the cov suite checks only the "No such file" substring, which an
    XX-wrapped msgid would still satisfy)."""
    monkeypatch.setattr("skit.tui_prefs.sys.platform", "win32")
    missing = tmp_path / "nope.exe"
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        screen.query_one("#pf-bash", Input).value = str(missing)
        await pilot.pause()
        screen.action_save()
        await pilot.pause()
        error = str(screen.query_one("#pf-bash-error", Static).render())
        assert isinstance(app.screen, PreferencesScreen)  # refused → stayed open
    assert error == f"No such file: {missing}"
    assert "XX" not in error


# ---------------------------------------------------------------------------
# action_install_skill: the exact "Installed…" notify copy
# ---------------------------------------------------------------------------


async def test_install_skill_notify_is_letter_exact(tmp_path, monkeypatch):
    """A successful install notifies letter-exact "Installed the skit Agent Skill: <path>"
    (the cov suite only checks the path is a substring, which an XX-wrapped / lower-cased
    msgid would still satisfy)."""
    from skit import agentskill

    target = agentskill.Target(name="claude", scope="user", base=tmp_path / ".claude")
    monkeypatch.setattr(agentskill, "detect_targets", lambda *, home, cwd: [target])
    notes: list[str] = []
    monkeypatch.setattr(
        PreferencesScreen, "notify", lambda self, message, **kw: notes.append(message)
    )
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        await _open_prefs(app)
        await pilot.pause()
        await pilot.press("ctrl+k")
        await pilot.pause()
        options = app.screen.query_one(OptionList)
        options.highlighted = 0
        options.action_select()
        await pilot.pause()
    written = tmp_path / ".claude" / "skills" / "skit" / "SKILL.md"
    assert notes == [f"Installed the skit Agent Skill: {written}"]


# ---------------------------------------------------------------------------
# action_close: the discard path dismisses exactly False (not None / True)
# ---------------------------------------------------------------------------


async def test_close_discard_path_dismisses_false(tmp_path):
    """When the screen is dirty and the user confirms discard, action_close dismisses with
    exactly False (nothing saved) — not None, not True. The cov suite only checks the screen
    closed, so the dismissed VALUE on this branch was untested."""
    from skit.tui_settings import DiscardChangesModal

    results: list[bool | None] = []
    app = tui.MenuApp()
    async with app.run_test() as pilot:
        screen = await _open_prefs(app, results.append)
        await pilot.pause()
        assert isinstance(screen, PreferencesScreen)
        list(screen.query_one("#pf-js", RadioSet).query(RadioButton))[2].value = True  # a real edit
        await pilot.pause()
        assert screen._dirty is True
        screen.action_close()  # dirty → ask
        await pilot.pause()
        modal = app.screen
        assert isinstance(modal, DiscardChangesModal)
        modal.action_discard()  # confirm discard → close
        await pilot.pause()
    assert results == [False]
