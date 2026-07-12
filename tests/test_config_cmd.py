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
        "form",
        "after_run",
        "shell.bash_path",
        "js.runner",
    }
    assert doc["mirror"] == "off"
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


# --- mirror ---


@pytest.mark.parametrize("name", list(config.PYPI_PRESETS))
def test_set_mirror_preset(name: str) -> None:
    result = runner.invoke(cli.app, ["config", "mirror", name])
    assert result.exit_code == 0
    assert config.load_mirror().pypi == config.PYPI_PRESETS[name]


def test_set_mirror_off() -> None:
    runner.invoke(cli.app, ["config", "mirror", "tsinghua"])
    result = runner.invoke(cli.app, ["config", "mirror", "off"])
    assert result.exit_code == 0
    assert not config.load_mirror().enabled


def test_unknown_mirror_exits_2() -> None:
    result = runner.invoke(cli.app, ["config", "mirror", "nope"])
    assert result.exit_code == 2


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
    runner.invoke(cli.app, ["config", "mirror", "tsinghua"])
    result = runner.invoke(cli.app, ["config", "lang", "auto"])
    assert result.exit_code == 0
    assert config.load_mirror().pypi == config.PYPI_PRESETS["tsinghua"]


def test_form_write_preserves_mirror_and_language() -> None:
    runner.invoke(cli.app, ["config", "lang", "zh-CN"])
    runner.invoke(cli.app, ["config", "mirror", "tsinghua"])
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


# --- the mirror wizard (first-run only now) ---


def test_mirror_wizard_preset(monkeypatch: pytest.MonkeyPatch) -> None:
    _prompts(monkeypatch, ["ustc"])
    cli._mirror_wizard()
    assert config.load_mirror().pypi == config.PYPI_PRESETS["ustc"]


def test_mirror_wizard_default_off_when_disabled(monkeypatch: pytest.MonkeyPatch) -> None:
    captured = _prompts(monkeypatch, ["off"])
    cli._mirror_wizard()
    assert captured[0]["default"] == "off"
    assert captured[0]["choices"] == [*config.PYPI_PRESETS, "custom", "off"]


def test_mirror_wizard_default_is_preset_when_enabled_preset(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config.save_mirror(config.preset("tsinghua"))
    captured = _prompts(monkeypatch, ["off"])
    cli._mirror_wizard()
    assert captured[0]["default"] == "tsinghua"
    assert not config.load_mirror().enabled


def test_mirror_wizard_default_custom_when_non_preset(monkeypatch: pytest.MonkeyPatch) -> None:
    config.save_mirror(config.MirrorConfig(enabled=True, pypi="https://old/simple"))
    captured = _prompts(monkeypatch, ["off"])
    cli._mirror_wizard()
    assert captured[0]["default"] == "custom"


def test_mirror_wizard_custom(monkeypatch: pytest.MonkeyPatch) -> None:
    config.save_mirror(config.MirrorConfig(enabled=True, pypi="https://old/simple"))
    captured = _prompts(
        monkeypatch, ["custom", "https://my/pypi", "https://my/py", "https://my/uv"]
    )
    cli._mirror_wizard()
    m = config.load_mirror()
    assert m.enabled
    assert (m.pypi, m.python_install, m.uv_binary) == (
        "https://my/pypi",
        "https://my/py",
        "https://my/uv",
    )
    assert captured[0]["default"] == "custom"


def test_mirror_wizard_custom_rejects_non_https_uv_binary(monkeypatch: pytest.MonkeyPatch) -> None:
    """An http:// uv-binary URL is rejected and re-prompted until it's https:// (that binary
    is downloaded and executed, so http would be a MITM->RCE vector)."""
    config.save_mirror(config.MirrorConfig(enabled=True, pypi="https://old/simple"))
    _prompts(
        monkeypatch,
        ["custom", "https://my/pypi", "https://my/py", "http://evil/uv", "https://good/uv"],
    )
    cli._mirror_wizard()
    assert config.load_mirror().uv_binary == "https://good/uv"


# --- first-run probe ---


def test_first_run_offers_and_configures_when_blocked(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(config, "looks_blocked", lambda: True)
    monkeypatch.setattr(cli.Confirm, "ask", lambda *_a, **_k: True)
    _prompts(monkeypatch, ["tsinghua"])
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
    _prompts(monkeypatch, ["tsinghua"])
    cli._maybe_first_run_setup()
    assert config.load_mirror().pypi == config.PYPI_PRESETS["tsinghua"]
