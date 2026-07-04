"""`skit config` command, the interactive wizard helpers, and the first-run mirror probe.

Non-interactive paths go through CliRunner; interactive branches (Prompt/Confirm, tty) are exercised
by calling the module functions directly with stubs — the same convention as test_cli.py.
"""

from __future__ import annotations

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
    """Stub Prompt.ask with canned answers, CAPTURING the default/choices of each call so tests can
    assert the wizard's computed defaults (not just its writes). Returns the capture list, appended
    to in call order."""
    it = iter(answers)
    captured: list[dict[str, object]] = []

    def fake_ask(*_a: object, **kw: object) -> str:
        captured.append({"default": kw.get("default"), "choices": kw.get("choices")})
        return next(it)

    monkeypatch.setattr(cli.Prompt, "ask", fake_ask)
    return captured


# --- `skit config` non-interactive (flags) ---


def test_show_reports_off_by_default() -> None:
    result = runner.invoke(cli.app, ["config", "--show"])
    assert result.exit_code == 0
    assert "off" in result.output.lower()


@pytest.mark.parametrize("name", list(config.PYPI_PRESETS))
def test_set_mirror_preset(name: str) -> None:
    result = runner.invoke(cli.app, ["config", "--mirror", name])
    assert result.exit_code == 0
    assert config.load_mirror().pypi == config.PYPI_PRESETS[name]


def test_set_mirror_off() -> None:
    runner.invoke(cli.app, ["config", "--mirror", "tsinghua"])
    result = runner.invoke(cli.app, ["config", "--mirror", "off"])
    assert result.exit_code == 0
    assert not config.load_mirror().enabled


def test_unknown_mirror_exits_2() -> None:
    result = runner.invoke(cli.app, ["config", "--mirror", "nope"])
    assert result.exit_code == 2


def test_set_lang_and_both_flags() -> None:
    result = runner.invoke(cli.app, ["config", "--lang", "zh-CN", "--mirror", "aliyun"])
    assert result.exit_code == 0
    assert config.load_config()["language"] == "zh-CN"
    assert config.load_mirror().pypi == config.PYPI_PRESETS["aliyun"]


def test_lang_auto_clears() -> None:
    runner.invoke(cli.app, ["config", "--lang", "zh-CN"])
    result = runner.invoke(cli.app, ["config", "--lang", "auto"])
    assert result.exit_code == 0
    assert "language" not in config.load_config()


def test_unknown_lang_exits_2() -> None:
    result = runner.invoke(cli.app, ["config", "--lang", "xx-YY"])
    assert result.exit_code == 2


# --- interactive wizard helpers ---


def test_language_wizard_sets_language(monkeypatch: pytest.MonkeyPatch) -> None:
    before = i18n.current_locale()  # "en" per the isolated fixture
    captured = _prompts(monkeypatch, ["zh-CN"])
    cli._language_wizard()
    assert config.load_config()["language"] == "zh-CN"
    # F4: the wizard offers the (pre-change) current locale as the default.
    assert captured[0]["default"] == before == "en"
    assert captured[0]["choices"] == ["auto", *i18n.available_locales()]


def test_language_wizard_auto(monkeypatch: pytest.MonkeyPatch) -> None:
    config.save_config({"language": "zh-CN"})
    _prompts(monkeypatch, ["auto"])
    cli._language_wizard()
    assert "language" not in config.load_config()


def test_mirror_wizard_preset(monkeypatch: pytest.MonkeyPatch) -> None:
    _prompts(monkeypatch, ["ustc"])
    cli._mirror_wizard()
    assert config.load_mirror().pypi == config.PYPI_PRESETS["ustc"]


def test_mirror_wizard_default_off_when_disabled(monkeypatch: pytest.MonkeyPatch) -> None:
    # F4: no saved mirror (disabled) -> the computed default is "off".
    captured = _prompts(monkeypatch, ["off"])
    cli._mirror_wizard()
    assert captured[0]["default"] == "off"
    assert captured[0]["choices"] == [*config.PYPI_PRESETS, "custom", "off"]


def test_mirror_wizard_default_is_preset_when_enabled_preset(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config.save_mirror(config.preset("tsinghua"))  # enabled + preset -> default is that preset key
    captured = _prompts(monkeypatch, ["off"])
    cli._mirror_wizard()
    # F4: the computed default reflects the saved preset (renamed from the old, misleading name).
    assert captured[0]["default"] == "tsinghua"
    assert not config.load_mirror().enabled  # answering "off" still disables


def test_mirror_wizard_default_custom_when_non_preset(monkeypatch: pytest.MonkeyPatch) -> None:
    # Enabled with a URL that matches no preset -> the computed default is "custom".
    config.save_mirror(config.MirrorConfig(enabled=True, pypi="https://old/simple"))
    captured = _prompts(monkeypatch, ["off"])
    cli._mirror_wizard()
    assert captured[0]["default"] == "custom"


def test_mirror_wizard_custom(monkeypatch: pytest.MonkeyPatch) -> None:
    config.save_mirror(
        config.MirrorConfig(enabled=True, pypi="https://old/simple")
    )  # -> "custom" default
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
    """F6: an http:// uv-binary URL is rejected and re-prompted until it's https:// (that binary is
    downloaded and executed, so http would be a MITM->RCE vector)."""
    config.save_mirror(config.MirrorConfig(enabled=True, pypi="https://old/simple"))
    # choice, pypi, python_install, uv_binary(http -> rejected), uv_binary(https -> accepted)
    _prompts(
        monkeypatch,
        ["custom", "https://my/pypi", "https://my/py", "http://evil/uv", "https://good/uv"],
    )
    cli._mirror_wizard()
    assert config.load_mirror().uv_binary == "https://good/uv"


def test_full_wizard_no_flags(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    _prompts(monkeypatch, ["auto", "tsinghua"])
    result = runner.invoke(cli.app, ["config"])
    assert result.exit_code == 0
    assert config.load_mirror().pypi == config.PYPI_PRESETS["tsinghua"]


def test_config_no_flags_non_interactive_prints_settings(monkeypatch: pytest.MonkeyPatch) -> None:
    """F10: with no flags and no tty, config falls back to printing settings — it must NOT run the
    Rich wizard (which would hang or read EOF on a pipe/CI)."""
    monkeypatch.setattr(cli, "_is_interactive", lambda: False)

    def _boom(*_a: object, **_k: object) -> str:
        raise AssertionError("the wizard must not run when non-interactive")

    monkeypatch.setattr(cli.Prompt, "ask", _boom)
    result = runner.invoke(cli.app, ["config"])
    assert result.exit_code == 0
    assert "off" in result.output.lower()  # settings were printed
    assert not config.mirror_configured()  # nothing was written


# --- (g) `skit config` flag writes preserve the other section ---


def test_mirror_off_preserves_language() -> None:
    runner.invoke(cli.app, ["config", "--lang", "zh-CN"])
    result = runner.invoke(cli.app, ["config", "--mirror", "off"])
    assert result.exit_code == 0
    assert config.load_config()["language"] == "zh-CN"  # language survives the mirror write


def test_lang_auto_preserves_mirror() -> None:
    runner.invoke(cli.app, ["config", "--mirror", "tsinghua"])
    result = runner.invoke(cli.app, ["config", "--lang", "auto"])
    assert result.exit_code == 0
    assert (
        config.load_mirror().pypi == config.PYPI_PRESETS["tsinghua"]
    )  # [mirror] survives lang clear


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
    assert config.is_configured()  # marker written so we don't probe again


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
    assert not probed  # never probed the network
    assert not config.is_configured()


def test_first_run_skipped_when_already_configured(monkeypatch: pytest.MonkeyPatch) -> None:
    config.disable()  # writes a [mirror] section -> mirror configured
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    probed: list[int] = []
    monkeypatch.setattr(config, "looks_blocked", lambda: probed.append(1) or True)
    cli._maybe_first_run_setup()
    assert not probed


def test_first_run_still_offered_after_language_only_config(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """F2: `skit lang zh-CN` writes config.toml (a language key) but NO [mirror] section, so the
    first-run mirror offer must still fire — the gate keys on mirror_configured(), not the file."""
    i18n.set_language("zh-CN")  # writes config.toml with only a language key
    assert config.is_configured()  # the file exists...
    assert not config.mirror_configured()  # ...but there's no [mirror] section yet
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)
    monkeypatch.setattr(config, "looks_blocked", lambda: True)
    monkeypatch.setattr(cli.Confirm, "ask", lambda *_a, **_k: True)
    _prompts(monkeypatch, ["tsinghua"])
    cli._maybe_first_run_setup()
    assert config.load_mirror().pypi == config.PYPI_PRESETS["tsinghua"]  # offer fired, mirror set
