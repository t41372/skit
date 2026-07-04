"""Config + mirror settings (config.py): persistence, presets, env injection with the defer rule."""

from __future__ import annotations

import socket
from pathlib import Path

import pytest

from skit import config


@pytest.fixture(autouse=True)
def cfg_dir(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
    return tmp_path


def test_defaults_when_no_config() -> None:
    assert not config.is_configured()
    assert config.load_config() == {}
    assert config.load_mirror() == config.MirrorConfig()
    assert config.mirror_env({}) == {}
    assert config.uv_binary_base() == ""


def test_preset_saves_all_three_vectors() -> None:
    config.save_mirror(config.preset("aliyun"))
    assert config.is_configured()
    m = config.load_mirror()
    assert m.enabled
    assert m.pypi == config.PYPI_PRESETS["aliyun"]
    assert m.python_install == config.PYTHON_INSTALL_MIRROR
    assert m.uv_binary == config.UV_BINARY_MIRROR


def test_save_mirror_preserves_other_keys() -> None:
    config.save_config({"language": "zh-CN"})
    config.save_mirror(config.preset("ustc"))
    doc = config.load_config()
    assert doc["language"] == "zh-CN"  # not clobbered by the mirror write
    assert doc["mirror"]["pypi"] == config.PYPI_PRESETS["ustc"]


def test_mirror_env_overlays_all_vectors() -> None:
    config.save_mirror(config.preset("tsinghua"))
    env = config.mirror_env({})
    assert env["UV_DEFAULT_INDEX"] == config.PYPI_PRESETS["tsinghua"]
    assert env["UV_PYTHON_INSTALL_MIRROR"] == config.PYTHON_INSTALL_MIRROR
    assert config.uv_binary_base() == config.UV_BINARY_MIRROR


@pytest.mark.parametrize("index_var", config._INDEX_ENV)
def test_mirror_env_defers_to_user_index(index_var: str) -> None:
    config.save_mirror(config.preset("tsinghua"))
    env = config.mirror_env({index_var: "https://mine/simple"})
    assert "UV_DEFAULT_INDEX" not in env  # the user's index wins
    assert "UV_PYTHON_INSTALL_MIRROR" in env  # the untouched vector is still injected


def test_mirror_env_defers_to_user_python_mirror() -> None:
    config.save_mirror(config.preset("tsinghua"))
    env = config.mirror_env({"UV_PYTHON_INSTALL_MIRROR": "https://mine/py/"})
    assert "UV_PYTHON_INSTALL_MIRROR" not in env
    assert "UV_DEFAULT_INDEX" in env


# --- (a) additive index vars must NOT trigger the defer (they don't replace uv's default index) ---


def test_mirror_env_does_not_defer_on_extra_index_url() -> None:
    config.save_mirror(config.preset("tsinghua"))
    env = config.mirror_env({"UV_EXTRA_INDEX_URL": "https://x"})
    # UV_EXTRA_INDEX_URL is additive, so the blocked default index is still live -> skit must inject.
    assert env["UV_DEFAULT_INDEX"] == config.PYPI_PRESETS["tsinghua"]


def test_mirror_env_does_not_defer_on_uv_index() -> None:
    config.save_mirror(config.preset("tsinghua"))
    env = config.mirror_env({"UV_INDEX": "https://x"})
    # UV_INDEX is additive too (F1: dropped from _INDEX_ENV), so injection must still happen.
    assert env["UV_DEFAULT_INDEX"] == config.PYPI_PRESETS["tsinghua"]


# --- (b) an empty-string user var means "unset": it must NOT suppress the mirror ---


def test_mirror_env_injects_when_index_env_blank() -> None:
    config.save_mirror(config.preset("tsinghua"))
    env = config.mirror_env({"UV_INDEX_URL": ""})
    assert env["UV_DEFAULT_INDEX"] == config.PYPI_PRESETS["tsinghua"]


def test_mirror_env_injects_when_python_mirror_blank() -> None:
    config.save_mirror(config.preset("tsinghua"))
    env = config.mirror_env({"UV_PYTHON_INSTALL_MIRROR": ""})
    assert env["UV_PYTHON_INSTALL_MIRROR"] == config.PYTHON_INSTALL_MIRROR


def test_disable_keeps_urls_but_turns_off() -> None:
    config.save_mirror(config.preset("tsinghua"))
    config.disable()
    m = config.load_mirror()
    assert not m.enabled
    assert m.pypi == config.PYPI_PRESETS["tsinghua"]  # URL retained for easy re-enable
    assert config.mirror_env({}) == {}
    assert config.uv_binary_base() == ""


def test_mirror_env_skips_empty_urls() -> None:
    # enabled but with blank URLs (e.g. hand-edited config): nothing to inject
    config.save_mirror(config.MirrorConfig(enabled=True))
    assert config.mirror_env({}) == {}


def test_load_mirror_ignores_malformed_section() -> None:
    config.save_config({"mirror": "not-a-table"})
    assert config.load_mirror() == config.MirrorConfig()


# --- (f) TOML type hardening: only a real bool enables; only str fields are URLs ---


def test_load_mirror_rejects_string_enabled() -> None:
    # A hand-edited `enabled = "false"` is a truthy string; it must NOT enable the mirror.
    config.save_config({"mirror": {"enabled": "false", "pypi": "https://x/simple"}})
    assert not config.load_mirror().enabled


def test_load_mirror_ignores_non_str_url() -> None:
    # A non-string URL (e.g. `pypi = 123`) must become blank, never the coerced string "123".
    config.save_config({"mirror": {"enabled": True, "pypi": 123}})
    m = config.load_mirror()
    assert m.enabled
    assert m.pypi == ""


# --- uv_binary is downloaded, chmod +x'd, and executed, so a hand-edited config must be https ---


def test_load_mirror_blanks_non_https_uv_binary() -> None:
    # A hand-edited http:// uv_binary bypasses the wizard's check; load_mirror must blank it so the
    # download falls back to the GitHub default rather than fetching an executable over plain http.
    config.save_config({"mirror": {"enabled": True, "uv_binary": "http://evil/uv"}})
    assert config.load_mirror().uv_binary == ""
    assert config.uv_binary_base() == ""  # -> uvman uses the GitHub default


def test_load_mirror_preserves_https_uv_binary() -> None:
    config.save_config({"mirror": {"enabled": True, "uv_binary": "https://ok/uv"}})
    assert config.load_mirror().uv_binary == "https://ok/uv"
    assert config.uv_binary_base() == "https://ok/uv"


def test_preset_uv_binary_stays_https() -> None:
    # Sanity: the shared NJU preset is https and must survive the https enforcement unchanged.
    config.save_mirror(config.preset("tsinghua"))
    assert config.load_mirror().uv_binary == config.UV_BINARY_MIRROR
    assert config.UV_BINARY_MIRROR.startswith("https://")


def test_load_config_tolerates_corrupt_toml(cfg_dir: Path) -> None:
    (cfg_dir / "config.toml").write_text("this is = = not [valid toml", encoding="utf-8")
    assert config.load_config() == {}


def test_looks_blocked_true_when_unreachable(monkeypatch: pytest.MonkeyPatch) -> None:
    def boom(*_a: object, **_k: object) -> object:
        raise OSError("blocked")

    monkeypatch.setattr(socket, "create_connection", boom)
    assert config.looks_blocked(timeout=0.01) is True


def test_looks_blocked_false_when_reachable(monkeypatch: pytest.MonkeyPatch) -> None:
    class FakeConn:
        def __enter__(self) -> FakeConn:
            return self

        def __exit__(self, *_a: object) -> bool:
            return False

    monkeypatch.setattr(socket, "create_connection", lambda *_a, **_k: FakeConn())
    assert config.looks_blocked(timeout=0.01) is False


# --- (d) looks_blocked short-circuits on the first unreachable host ---


class _FakeConn:
    def __enter__(self) -> _FakeConn:
        return self

    def __exit__(self, *_a: object) -> bool:
        return False


def test_looks_blocked_short_circuits_on_first_host(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[object] = []

    def boom(addr: tuple[str, int], timeout: float | None = None) -> object:
        calls.append(addr[0])
        raise OSError("blocked")

    monkeypatch.setattr(socket, "create_connection", boom)
    assert config.looks_blocked(timeout=0.01) is True
    # First host (pypi.org) is unreachable -> return immediately, never probe github.com.
    assert calls == ["pypi.org"]


def test_looks_blocked_true_when_second_host_unreachable(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[str] = []

    def conn(addr: tuple[str, int], timeout: float | None = None) -> object:
        seen.append(addr[0])
        if addr[0] == "pypi.org":
            return _FakeConn()  # first host reachable
        raise OSError("blocked")  # github.com unreachable

    monkeypatch.setattr(socket, "create_connection", conn)
    assert config.looks_blocked(timeout=0.01) is True
    assert seen == ["pypi.org", "github.com"]
