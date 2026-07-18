"""`skit config` (git-config grammar: bare = list, KEY = read, KEY VALUE = write),
the first-run mirror offer, and the mirror wizard it still uses.

Non-interactive paths go through CliRunner; interactive branches (Prompt/Confirm, tty)
are exercised by calling the module functions directly with stubs — the same convention
as test_cli.py.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, config, i18n

runner = CliRunner()


@pytest.fixture(autouse=True)
def isolated(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")
    yield
    i18n.init("en")


def _prompts(monkeypatch: pytest.MonkeyPatch, answers: list[str]) -> list[dict[str, object]]:
    """Stub Prompt.ask with canned answers, CAPTURING the default/choices of each call."""
    it = iter(answers)
    captured: list[dict[str, object]] = []

    def fake_ask(*_a: object, **kw: object) -> str:
        captured.append({"default": kw.get("default"), "choices": kw.get("choices")})
        return next(it)

    monkeypatch.setattr(cli.Prompt, "ask", fake_ask)
    return captured


# --- bare `skit config`: list everything ---


def test_bare_config_lists_all_keys() -> None:
    result = runner.invoke(cli.app, ["config"])
    assert result.exit_code == 0
    for key in ("lang", "editor", "mirror", "form", "after_run"):
        assert key in result.output
    assert "off" in result.output  # mirror default
    assert "tui" in result.output  # form default


def test_bare_config_json() -> None:
    result = runner.invoke(cli.app, ["config", "--json"])
    assert result.exit_code == 0
    doc = json.loads(result.output)
    assert set(doc) == {
        "lang",
        "editor",
        "mirror",
        "mirror.pypi",
        "mirror.github",
        "mirror.npm",
        "form",
        "after_run",
        "shell.bash_path",
        "js.runner",
    }
    assert doc["mirror"] == "off"
    assert doc["mirror.pypi"] == "off"
    assert doc["mirror.github"] == "off"
    assert doc["mirror.npm"] == "off"
    assert doc["form"] == "tui"
    assert doc["after_run"] == "exit"  # the launcher default


def test_unknown_key_exits_2() -> None:
    result = runner.invoke(cli.app, ["config", "theme"])
    assert result.exit_code == 2


# --- lang ---


def test_set_lang_writes_language_key() -> None:
    result = runner.invoke(cli.app, ["config", "lang", "zh-CN"])
    assert result.exit_code == 0
    assert config.load_config()["language"] == "zh-CN"


def test_read_lang_shows_override() -> None:
    runner.invoke(cli.app, ["config", "lang", "zh-CN"])
    result = runner.invoke(cli.app, ["config", "lang"])
    assert result.exit_code == 0
    assert "zh-CN" in result.output


def test_lang_auto_clears() -> None:
    runner.invoke(cli.app, ["config", "lang", "zh-CN"])
    result = runner.invoke(cli.app, ["config", "lang", "auto"])
    assert result.exit_code == 0
    assert "language" not in config.load_config()


def test_unknown_lang_exits_2() -> None:
    result = runner.invoke(cli.app, ["config", "lang", "xx-YY"])
    assert result.exit_code == 2


# --- mirror: three independent per-ecosystem axes + a master switch ---


@pytest.mark.parametrize("name", list(config.PYPI_PRESETS))
def test_set_mirror_pypi_preset(name: str) -> None:
    result = runner.invoke(cli.app, ["config", "mirror.pypi", name])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert m.enabled
    assert m.pypi == config.PYPI_PRESETS[name]


def test_pypi_axis_does_not_drag_other_axes() -> None:
    """The regression this design fix exists for: a PyPI vendor choice must configure the
    PyPI axis and NOTHING else — the npm/github axes have their own vendor landscapes."""
    runner.invoke(cli.app, ["config", "mirror.pypi", "tsinghua"])
    m = config.load_mirror()
    assert (m.npm, m.python_install, m.uv_binary) == ("", "", "")


def test_set_mirror_npm_alone() -> None:
    result = runner.invoke(cli.app, ["config", "mirror.npm", "npmmirror"])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert m.enabled
    assert m.npm == config.NPM_REGISTRY_MIRROR
    assert (m.pypi, m.python_install, m.uv_binary) == ("", "", "")


def test_set_mirror_github_expands_both_urls() -> None:
    result = runner.invoke(cli.app, ["config", "mirror.github", "nju"])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert m.python_install == config.PYTHON_INSTALL_MIRROR
    assert m.uv_binary == config.UV_BINARY_MIRROR
    assert (m.pypi, m.npm) == ("", "")


def test_set_mirror_github_custom_base_expands() -> None:
    result = runner.invoke(cli.app, ["config", "mirror.github", "https://my.mirror/gh"])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert m.python_install == "https://my.mirror/gh/astral-sh/python-build-standalone/"
    assert m.uv_binary == "https://my.mirror/gh/astral-sh/uv"


def test_set_mirror_github_off_clears_both_urls() -> None:
    runner.invoke(cli.app, ["config", "mirror.github", "nju"])
    runner.invoke(cli.app, ["config", "mirror.npm", "npmmirror"])
    result = runner.invoke(cli.app, ["config", "mirror.github", "off"])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert (m.python_install, m.uv_binary) == ("", "")
    assert m.enabled  # the npm axis is still on
    assert m.npm == config.NPM_REGISTRY_MIRROR
    # The RAW doc must hold "" too: load_mirror() blanks a non-https uv_binary on read, so a
    # write that dropped garbage into the file would be invisible above — check the disk.
    doc = config.load_config()
    assert doc["mirror"]["python_install"] == ""
    assert doc["mirror"]["uv_binary"] == ""


def test_paused_github_write_prints_notice_and_clear_does_not() -> None:
    """F2 for the github axis: a URL-storing write under pause emits the stderr notice and
    stays paused; the clearing write emits none and leaves the other paused axes alone."""
    runner.invoke(cli.app, ["config", "mirror.pypi", "tsinghua"])
    runner.invoke(cli.app, ["config", "mirror", "off"])  # pause
    result = runner.invoke(cli.app, ["config", "mirror.github", "nju"])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert not m.enabled  # still paused
    assert m.python_install == config.PYTHON_INSTALL_MIRROR  # the write landed
    assert m.pypi == config.PYPI_PRESETS["tsinghua"]  # the paused axis survived
    assert "switched off" in " ".join(result.stderr.split())
    result = runner.invoke(cli.app, ["config", "mirror.github", "off"])
    assert result.exit_code == 0
    assert "switched off" not in " ".join(result.output.split())  # a clear is not a URL write
    m = config.load_mirror()
    assert (m.python_install, m.uv_binary) == ("", "")
    assert m.pypi == config.PYPI_PRESETS["tsinghua"]


def test_set_mirror_github_rejects_http_base() -> None:
    # The base derives the uv-binary URL (downloaded and executed) -> https only.
    result = runner.invoke(cli.app, ["config", "mirror.github", "http://evil/gh"])
    assert result.exit_code == 2
    assert config.load_mirror().uv_binary == ""


def test_set_mirror_axis_custom_url() -> None:
    result = runner.invoke(cli.app, ["config", "mirror.pypi", "https://my.index/simple"])
    assert result.exit_code == 0
    assert config.load_mirror().pypi == "https://my.index/simple"


def test_set_mirror_axis_off_keeps_the_others() -> None:
    runner.invoke(cli.app, ["config", "mirror.pypi", "tsinghua"])
    runner.invoke(cli.app, ["config", "mirror.npm", "npmmirror"])
    result = runner.invoke(cli.app, ["config", "mirror.pypi", "off"])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert m.enabled
    assert m.pypi == ""
    assert m.npm == config.NPM_REGISTRY_MIRROR


def test_set_last_axis_off_disables() -> None:
    runner.invoke(cli.app, ["config", "mirror.npm", "npmmirror"])
    runner.invoke(cli.app, ["config", "mirror.npm", "off"])
    assert not config.load_mirror().enabled


@pytest.mark.parametrize("key", ["mirror.pypi", "mirror.npm"])
def test_unknown_axis_value_exits_2(key: str) -> None:
    # A typo (or another axis's vendor name) must never be saved as a bogus URL.
    result = runner.invoke(cli.app, ["config", key, "tsnighua"])
    assert result.exit_code == 2
    assert not config.load_mirror().enabled


def test_npm_axis_rejects_pypi_vendor_name() -> None:
    # The old single-axis grammar's core lie, now a hard error: PyPI vendors aren't npm vendors.
    result = runner.invoke(cli.app, ["config", "mirror.npm", "tsinghua"])
    assert result.exit_code == 2


def test_mirror_master_off_preserves_urls_and_on_restores() -> None:
    runner.invoke(cli.app, ["config", "mirror.pypi", "tsinghua"])
    result = runner.invoke(cli.app, ["config", "mirror", "off"])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert not m.enabled
    assert m.pypi == config.PYPI_PRESETS["tsinghua"]  # kept for the return trip
    result = runner.invoke(cli.app, ["config", "mirror", "on"])
    assert result.exit_code == 0
    assert config.load_mirror().enabled


def test_mirror_master_on_with_nothing_saved_exits_2() -> None:
    result = runner.invoke(cli.app, ["config", "mirror", "on"])
    assert result.exit_code == 2
    assert "mirror.pypi" in result.output  # points at the axis keys


def test_mirror_master_rejects_vendor_names_with_axis_pointer() -> None:
    """`skit config mirror tsinghua` (the old grammar) must fail loudly and point at
    mirror.pypi — a vendor name at the master level would have to guess the other axes."""
    result = runner.invoke(cli.app, ["config", "mirror", "tsinghua"])
    assert result.exit_code == 2
    assert "mirror.pypi" in result.output
    assert not config.load_mirror().enabled


def test_paused_axis_write_preserves_other_axes_and_stays_paused() -> None:
    """F2: after `mirror off` the config is PAUSED (URLs kept, master off). Writing another
    axis must preserve the paused axes' URLs, keep the master off, and print the re-enable
    notice — never silently destroy the paused URL nor resurrect every axis behind the user."""
    runner.invoke(cli.app, ["config", "mirror.pypi", "tsinghua"])
    runner.invoke(cli.app, ["config", "mirror", "off"])
    result = runner.invoke(cli.app, ["config", "mirror.npm", "npmmirror"])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert not m.enabled  # stays paused, not silently re-enabled
    assert m.pypi == config.PYPI_PRESETS["tsinghua"]  # the paused axis survived the write
    assert m.npm == config.NPM_REGISTRY_MIRROR  # the asked-for axis landed
    # R2-2: the notice is a skit-side signal and goes to STDERR — an agent piping stdout
    # must see only the confirmation line, never a mixed-in warning.
    assert "switched off" in " ".join(result.stderr.split())
    stdout = " ".join(result.stdout.split())
    assert "switched off" not in stdout
    assert "mirror.npm = npmmirror" in stdout  # stdout carries only the confirmation


def test_paused_axis_clear_leaves_other_axes_and_prints_no_notice() -> None:
    """F2 (cont.): under a paused config, clearing one axis (`mirror.npm off`) leaves the
    other paused axes' URLs intact — and a clear writes no URL, so no re-enable notice."""
    runner.invoke(cli.app, ["config", "mirror.pypi", "tsinghua"])
    runner.invoke(cli.app, ["config", "mirror.npm", "npmmirror"])
    runner.invoke(cli.app, ["config", "mirror", "off"])  # pause both axes
    result = runner.invoke(cli.app, ["config", "mirror.npm", "off"])
    assert result.exit_code == 0
    m = config.load_mirror()
    assert not m.enabled
    assert m.pypi == config.PYPI_PRESETS["tsinghua"]  # untouched
    assert m.npm == ""  # cleared
    assert "switched off" not in " ".join(result.output.split())


def test_paused_config_is_fully_visible_in_config_list() -> None:
    """F4: a paused config stays legible key-by-key in `skit config` — the master reads off
    while each axis key still shows its stored token, so nothing is silently hidden."""
    runner.invoke(cli.app, ["config", "mirror.pypi", "aliyun"])
    runner.invoke(cli.app, ["config", "mirror.npm", "npmmirror"])
    runner.invoke(cli.app, ["config", "mirror", "off"])  # pause
    result = runner.invoke(cli.app, ["config"])
    assert result.exit_code == 0
    flat = " ".join(result.output.split())
    assert "mirror off" in flat  # the master reads off (paused)
    assert "mirror.pypi aliyun" in flat  # the stored axes are still spelled out, one per key
    assert "mirror.github off" in flat
    assert "mirror.npm npmmirror" in flat


def test_read_mirror_axis_shows_custom_url() -> None:
    runner.invoke(cli.app, ["config", "mirror.npm", "https://my.registry"])
    result = runner.invoke(cli.app, ["config", "mirror.npm"])
    assert "https://my.registry" in result.output.replace("\n", "")


@pytest.mark.parametrize("value", ["nju", "https://my.mirror/gh"])
def test_mirror_github_read_value_round_trips(value: str) -> None:
    """F3: the value `config mirror.github` prints (a preset name or a base URL) writes
    straight back at exit 0 with no state change — read/write vocabulary is symmetric."""
    runner.invoke(cli.app, ["config", "mirror.github", value])
    shown = runner.invoke(cli.app, ["config", "mirror.github"]).output.strip()
    before = config.load_mirror()
    result = runner.invoke(cli.app, ["config", "mirror.github", shown])
    assert result.exit_code == 0, (value, shown, result.output)
    assert config.load_mirror() == before


@pytest.mark.parametrize(
    "bad", ["https://a b/gh", "https://a·b/gh", "https://x/py/ + https://x/uv"]
)
def test_mirror_github_rejects_display_strings(bad: str) -> None:
    """F3: a display string (embedded space, or the " · "/" + " summary separators) must fail
    loudly at exit 2 and never land on disk as a garbage URL."""
    runner.invoke(cli.app, ["config", "mirror.github", "nju"])
    before = config.load_mirror()
    result = runner.invoke(cli.app, ["config", "mirror.github", bad])
    assert result.exit_code == 2
    assert config.load_mirror() == before  # disk untouched


def test_mirror_axis_rejects_whitespace_url() -> None:
    """The single-URL axes (pypi/npm) reject a whitespace-bearing token the same way: it is
    neither a preset nor a pastable URL, so exit 2 and nothing persisted."""
    result = runner.invoke(cli.app, ["config", "mirror.pypi", "https://a b/simple"])
    assert result.exit_code == 2
    assert config.load_mirror().pypi == ""


# --- config KEY --json (single-key raw machine tokens, item #11) ---


def test_config_json_single_key_is_raw_master_token() -> None:
    runner.invoke(cli.app, ["config", "mirror.pypi", "tsinghua"])
    doc = json.loads(runner.invoke(cli.app, ["config", "mirror", "--json"]).output)
    assert doc == {"mirror": "on"}


def test_config_json_single_key_lang_is_raw_override_tag() -> None:
    runner.invoke(cli.app, ["config", "lang", "zh-CN"])
    doc = json.loads(runner.invoke(cli.app, ["config", "lang", "--json"]).output)
    assert doc == {"lang": "zh-CN"}  # the raw override tag, not "auto (…)" prose


def test_config_json_lang_unset_is_empty_string() -> None:
    doc = json.loads(runner.invoke(cli.app, ["config", "lang", "--json"]).output)
    assert doc == {"lang": ""}  # unset/auto reads as "", never a localized fallback string


def test_lang_override_non_string_reads_as_empty() -> None:
    # _config_raw_value("lang") must never leak a non-str into the JSON contract, even if
    # config.toml was hand-corrupted to a non-string language.
    config.save_config({"language": 123})
    assert cli._lang_override() == ""


@pytest.mark.parametrize(
    ("setup", "expected"),
    [
        (["config", "mirror.github", "nju"], "nju"),
        (["config", "mirror.github", "https://my/gh"], "https://my/gh"),
    ],
)
def test_config_json_mirror_github_raw_token(setup: list[str], expected: str) -> None:
    runner.invoke(cli.app, setup)
    doc = json.loads(runner.invoke(cli.app, ["config", "mirror.github", "--json"]).output)
    assert doc == {"mirror.github": expected}


def test_config_json_mirror_github_underivable_pair_is_literal_custom() -> None:
    """Item #11: a hand-edited github pair no base derives reads back as the literal token
    "custom" — a value that fails loudly if written back, never display prose saved as a URL."""
    config.save_mirror(config.compose(python_install="https://my/py/", uv_binary="https://my/uv"))
    doc = json.loads(runner.invoke(cli.app, ["config", "mirror.github", "--json"]).output)
    assert doc == {"mirror.github": "custom"}


# --- editor ---


def test_set_editor() -> None:
    result = runner.invoke(cli.app, ["config", "editor", "code --wait"])
    assert result.exit_code == 0
    assert config.load_editor() == "code --wait"
    assert "code --wait" in result.output  # confirmation echoes the new value


def test_clear_editor_with_empty_value() -> None:
    runner.invoke(cli.app, ["config", "editor", "nano"])
    result = runner.invoke(cli.app, ["config", "editor", ""])
    assert result.exit_code == 0
    assert config.load_editor() == ""


def test_read_editor_default_line() -> None:
    result = runner.invoke(cli.app, ["config", "editor"])
    assert result.exit_code == 0
    assert "$VISUAL / $EDITOR" in result.output


# --- form ---


def test_form_defaults_to_tui() -> None:
    result = runner.invoke(cli.app, ["config", "form"])
    assert result.exit_code == 0
    assert "tui" in result.output


def test_set_form_plain_and_back() -> None:
    result = runner.invoke(cli.app, ["config", "form", "plain"])
    assert result.exit_code == 0
    assert config.load_form() == "plain"
    runner.invoke(cli.app, ["config", "form", "tui"])
    assert config.load_form() == "tui"


def test_unknown_form_style_exits_2() -> None:
    result = runner.invoke(cli.app, ["config", "form", "fancy"])
    assert result.exit_code == 2


# --- after_run ---


def test_read_after_run_default() -> None:
    result = runner.invoke(cli.app, ["config", "after_run"])
    assert result.exit_code == 0
    assert "exit" in result.output  # the launcher default


def test_set_after_run_stay_and_back() -> None:
    result = runner.invoke(cli.app, ["config", "after_run", "stay"])
    assert result.exit_code == 0
    assert config.load_after_run() == "stay"
    runner.invoke(cli.app, ["config", "after_run", "exit"])
    assert config.load_after_run() == "exit"


def test_unknown_after_run_exits_2() -> None:
    result = runner.invoke(cli.app, ["config", "after_run", "loop"])
    assert result.exit_code == 2


def test_after_run_garbage_in_config_file_normalizes_to_exit() -> None:
    doc = config.load_config()
    doc["after_run"] = "never"
    config.save_config(doc)
    assert config.load_after_run() == "exit"


# --- writes preserve the other sections ---


def test_mirror_write_preserves_language() -> None:
    runner.invoke(cli.app, ["config", "lang", "zh-CN"])
    result = runner.invoke(cli.app, ["config", "mirror", "off"])
    assert result.exit_code == 0
    assert config.load_config()["language"] == "zh-CN"


def test_lang_clear_preserves_mirror() -> None:
    runner.invoke(cli.app, ["config", "mirror.pypi", "tsinghua"])
    result = runner.invoke(cli.app, ["config", "lang", "auto"])
    assert result.exit_code == 0
    assert config.load_mirror().pypi == config.PYPI_PRESETS["tsinghua"]


def test_form_write_preserves_mirror_and_language() -> None:
    runner.invoke(cli.app, ["config", "lang", "zh-CN"])
    runner.invoke(cli.app, ["config", "mirror.pypi", "tsinghua"])
    runner.invoke(cli.app, ["config", "form", "plain"])
    assert config.load_config()["language"] == "zh-CN"
    assert config.load_mirror().enabled
    assert config.load_form() == "plain"


# --- shell.bash_path ---


def test_read_bash_path_default_line() -> None:
    result = runner.invoke(cli.app, ["config", "shell.bash_path"])
    assert result.exit_code == 0
    assert "auto" in result.output  # the unset placeholder


def test_set_bash_path_to_existing_file(tmp_path: Path) -> None:
    bash = tmp_path / "bash"
    bash.write_text("", encoding="utf-8")
    result = runner.invoke(cli.app, ["config", "shell.bash_path", str(bash)])
    assert result.exit_code == 0, result.output
    assert config.load_bash_path() == str(bash)
    # rich soft-wraps the long path; flatten before matching the echoed confirmation.
    assert str(bash) in result.output.replace("\n", "")


def test_set_bash_path_to_missing_file_is_usage_error(tmp_path: Path) -> None:
    ghost = tmp_path / "nope"  # never created
    result = runner.invoke(cli.app, ["config", "shell.bash_path", str(ghost)])
    assert result.exit_code == 2
    assert config.load_bash_path() == ""  # nothing written on the rejection


def test_clear_bash_path_with_empty_value(tmp_path: Path) -> None:
    bash = tmp_path / "bash"
    bash.write_text("", encoding="utf-8")
    runner.invoke(cli.app, ["config", "shell.bash_path", str(bash)])
    result = runner.invoke(cli.app, ["config", "shell.bash_path", ""])
    assert result.exit_code == 0
    assert config.load_bash_path() == ""  # empty clears, no existence check


def test_bare_config_lists_dotted_keys() -> None:
    result = runner.invoke(cli.app, ["config"])
    assert result.exit_code == 0
    assert "shell.bash_path" in result.output
    assert "js.runner" in result.output


# --- js.runner ---


def test_read_js_runner_default_line() -> None:
    result = runner.invoke(cli.app, ["config", "js.runner"])
    assert result.exit_code == 0
    assert "auto" in result.output


@pytest.mark.parametrize("name", list(config.JS_RUNNERS))
def test_set_js_runner(name: str) -> None:
    result = runner.invoke(cli.app, ["config", "js.runner", name])
    assert result.exit_code == 0, result.output
    assert config.load_js_runner() == name
    assert name in result.output


def test_set_js_runner_unknown_is_usage_error() -> None:
    result = runner.invoke(cli.app, ["config", "js.runner", "carrier-pigeon"])
    assert result.exit_code == 2
    # the error names the real choices so the user can fix it in one step
    assert "deno" in result.output
    assert config.load_js_runner() == ""


def test_clear_js_runner_with_empty_value() -> None:
    runner.invoke(cli.app, ["config", "js.runner", "bun"])
    result = runner.invoke(cli.app, ["config", "js.runner", ""])
    assert result.exit_code == 0
    assert config.load_js_runner() == ""


# --- the mirror wizard (first-run only now): one question per ecosystem axis ---


def test_mirror_wizard_asks_one_question_per_axis(monkeypatch: pytest.MonkeyPatch) -> None:
    captured = _prompts(monkeypatch, ["ustc", "nju", "npmmirror"])
    cli._mirror_wizard()
    assert captured[0]["choices"] == [*config.PYPI_PRESETS, "custom", "off"]
    assert captured[1]["choices"] == [*config.GITHUB_RELEASE_PRESETS, "custom", "off"]
    assert captured[2]["choices"] == [*config.NPM_PRESETS, "custom", "off"]
    m = config.load_mirror()
    assert m.pypi == config.PYPI_PRESETS["ustc"]
    assert m.python_install == config.PYTHON_INSTALL_MIRROR
    assert m.uv_binary == config.UV_BINARY_MIRROR
    assert m.npm == config.NPM_REGISTRY_MIRROR


def test_mirror_wizard_defaults_are_the_recommended_presets(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """First run: the user just opted into mirror setup, so each axis defaults to its own
    recommended preset (three Enters = tsinghua + nju + npmmirror), never a buried "off"."""
    captured = _prompts(monkeypatch, ["off", "off", "off"])
    cli._mirror_wizard()
    assert captured[0]["default"] == "tsinghua"
    assert captured[1]["default"] == "nju"
    assert captured[2]["default"] == "npmmirror"
    assert not config.load_mirror().enabled  # explicit off x3 still means off


def test_mirror_wizard_axes_answer_independently(monkeypatch: pytest.MonkeyPatch) -> None:
    # npm-only: the other axes' "off" answers must not leak any vendor onto them.
    _prompts(monkeypatch, ["off", "off", "npmmirror"])
    cli._mirror_wizard()
    m = config.load_mirror()
    assert m.enabled
    assert m.npm == config.NPM_REGISTRY_MIRROR
    assert (m.pypi, m.python_install, m.uv_binary) == ("", "", "")


def test_mirror_wizard_default_ignores_saved_choice(monkeypatch: pytest.MonkeyPatch) -> None:
    """The wizard runs only on an unconfigured store, so its PyPI default is unconditionally
    the recommended preset — a previously saved choice must NOT leak in (dead code removed)."""
    config.save_mirror(config.compose(pypi=config.PYPI_PRESETS["aliyun"]))
    captured = _prompts(monkeypatch, ["off", "off", "off"])
    cli._mirror_wizard()
    assert captured[0]["default"] == "tsinghua"  # not "aliyun"
    assert not config.load_mirror().enabled


def test_mirror_wizard_default_ignores_non_preset_saved_url(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A saved non-preset URL must not surface as a "custom" default either — the default is
    always the recommended preset, never computed from current state."""
    config.save_mirror(config.compose(pypi="https://old/simple"))
    captured = _prompts(monkeypatch, ["off", "off", "off"])
    cli._mirror_wizard()
    assert captured[0]["default"] == "tsinghua"  # not "custom"


def test_mirror_wizard_custom(monkeypatch: pytest.MonkeyPatch) -> None:
    config.save_mirror(config.compose(pypi="https://old/simple"))
    captured = _prompts(
        monkeypatch,
        ["custom", "https://my/pypi", "custom", "https://my/gh", "custom", "https://my/npm"],
    )
    cli._mirror_wizard()
    m = config.load_mirror()
    assert m.enabled
    # The github axis asks one base URL; it derives both github-release vectors.
    assert (m.pypi, m.python_install, m.uv_binary, m.npm) == (
        "https://my/pypi",
        "https://my/gh/astral-sh/python-build-standalone/",
        "https://my/gh/astral-sh/uv",
        "https://my/npm",
    )
    assert captured[0]["default"] == "tsinghua"  # the choice default, not the saved URL
    assert captured[1]["default"] is None  # _prompt_axis_url carries no preset default


def test_mirror_wizard_custom_rejects_non_https_github_base(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The github base derives the uv-binary download (executed), so an http:// base is
    rejected and re-prompted until it's https:// (http would be a MITM->RCE vector)."""
    config.save_mirror(config.compose(pypi="https://old/simple"))
    _prompts(
        monkeypatch,
        [
            "custom",
            "https://my/pypi",
            "custom",
            "http://evil/gh",
            "https://good/gh",
            "custom",
            "https://my/npm",
        ],
    )
    cli._mirror_wizard()
    m = config.load_mirror()
    assert m.python_install == "https://good/gh/astral-sh/python-build-standalone/"
    assert m.uv_binary == "https://good/gh/astral-sh/uv"
    assert m.npm == "https://my/npm"


@pytest.mark.parametrize("bad", ["", "tsinghua", "https://a b/simple"])
def test_mirror_wizard_custom_axis_bad_url_reprompts(
    monkeypatch: pytest.MonkeyPatch, bad: str
) -> None:
    """R2-4: the wizard's custom-URL prompt applies the same is_url_token gate as the CLI
    axis keys — empty, a vendor-name typo ("tsinghua" would persist as a broken
    UV_DEFAULT_INDEX), or display prose all loop until a real one-token URL arrives."""
    _prompts(monkeypatch, ["custom", bad, "https://my/pypi", "off", "off"])
    cli._mirror_wizard()
    assert config.load_mirror().pypi == "https://my/pypi"


# --- first-run probe ---


def test_first_run_offers_and_configures_when_blocked(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(config, "looks_blocked", lambda: True)
    monkeypatch.setattr(cli.Confirm, "ask", lambda *_a, **_k: True)
    _prompts(monkeypatch, ["tsinghua", "nju", "npmmirror"])
    cli._maybe_first_run_setup()
    assert config.load_mirror().pypi == config.PYPI_PRESETS["tsinghua"]
    assert config.is_configured()


def test_first_run_declined_still_marks_done(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(config, "looks_blocked", lambda: True)
    monkeypatch.setattr(cli.Confirm, "ask", lambda *_a, **_k: False)
    cli._maybe_first_run_setup()
    assert not config.load_mirror().enabled
    assert config.is_configured()


def test_first_run_not_blocked_marks_done(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(config, "looks_blocked", lambda: False)
    cli._maybe_first_run_setup()
    assert config.is_configured()
    assert not config.load_mirror().enabled


def test_first_run_skipped_when_not_interactive(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(cli, "_is_interactive", lambda: False)
    probed: list[int] = []
    monkeypatch.setattr(config, "looks_blocked", lambda: probed.append(1) or True)
    cli._maybe_first_run_setup()
    assert not probed
    assert not config.is_configured()


def test_first_run_skipped_when_already_configured(monkeypatch: pytest.MonkeyPatch) -> None:
    config.disable()
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    probed: list[int] = []
    monkeypatch.setattr(config, "looks_blocked", lambda: probed.append(1) or True)
    cli._maybe_first_run_setup()
    assert not probed


def test_first_run_still_offered_after_language_only_config(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """`skit config lang zh-CN` writes config.toml (a language key) but NO [mirror] section,
    so the first-run mirror offer must still fire."""
    i18n.set_language("zh-CN")
    assert config.is_configured()
    assert not config.mirror_configured()
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(config, "looks_blocked", lambda: True)
    monkeypatch.setattr(cli.Confirm, "ask", lambda *_a, **_k: True)
    _prompts(monkeypatch, ["tsinghua", "nju", "npmmirror"])
    cli._maybe_first_run_setup()
    assert config.load_mirror().pypi == config.PYPI_PRESETS["tsinghua"]
