"""Config + mirror settings (config.py): persistence, per-axis presets, env injection with
the defer rule. The three mirror axes (pypi / github-release / npm) are independent — each
ecosystem has its own vendor landscape, and these tests pin that independence."""

from __future__ import annotations

import socket
from pathlib import Path

import pytest

from conftest import full_mirror
from skit import atomic, config


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


def test_full_mirror_saves_all_four_vectors() -> None:
    config.save_mirror(full_mirror())
    assert config.is_configured()
    m = config.load_mirror()
    assert m.enabled
    assert m.pypi == config.PYPI_PRESETS["tsinghua"]
    assert m.python_install == config.PYTHON_INSTALL_MIRROR
    assert m.uv_binary == config.UV_BINARY_MIRROR
    assert m.npm == config.NPM_REGISTRY_MIRROR


def test_compose_enables_iff_any_axis_on() -> None:
    assert not config.compose().enabled
    for axis in ("pypi", "python_install", "uv_binary", "npm"):
        assert config.compose(**{axis: "https://x"}).enabled, axis


def test_axes_are_independent() -> None:
    """One axis's vendor choice must never drag another axis along: the PyPI providers are
    not npm or github-release vendors, and each axis works alone."""
    m = config.compose(pypi=config.PYPI_PRESETS["aliyun"])
    assert (m.python_install, m.uv_binary, m.npm) == ("", "", "")
    m = config.compose(npm=config.NPM_PRESETS["npmmirror"])
    assert (m.pypi, m.python_install, m.uv_binary) == ("", "", "")
    base = config.GITHUB_RELEASE_PRESETS["nju"]
    m = config.compose(python_install=config.github_release_urls(base)[0])
    assert (m.pypi, m.npm) == ("", "")


def test_is_url_token_accepts_pastable_http_urls() -> None:
    # The shared custom-URL gate (CLI axis keys, TUI inputs, wizard prompts all route here).
    assert config.is_url_token("https://pypi.tuna.tsinghua.edu.cn/simple") is True
    assert config.is_url_token("http://corp.internal/simple") is True  # http allowed (pypi/npm)


@pytest.mark.parametrize(
    "bad",
    [
        "",  # empty
        "tsinghua",  # a vendor name, not a URL — would persist as a broken UV_DEFAULT_INDEX
        "ftp://x",  # non-http(s) scheme
        "https://a b/x",  # embedded space (display prose)
        "https://a\tb",  # any whitespace, not just spaces
        "https://a\nb",
        "https://a·b",  # the axes_summary display separator
        "pypi=tsinghua · npm=off",  # a round-tripped display string
    ],
)
def test_is_url_token_rejects_non_urls(bad: str) -> None:
    assert config.is_url_token(bad) is False


def test_github_release_urls_expand_from_one_base() -> None:
    python_install, uv_binary = config.github_release_urls("https://my.mirror/gh/")
    assert python_install == "https://my.mirror/gh/astral-sh/python-build-standalone/"
    assert uv_binary == "https://my.mirror/gh/astral-sh/uv"


def test_axis_choice_readers() -> None:
    m = full_mirror()
    assert config.pypi_choice(m) == "tsinghua"
    assert config.github_choice(m) == "nju"
    assert config.npm_choice(m) == "npmmirror"
    custom = config.compose(
        pypi="https://my/simple", python_install="https://my/py/", npm="https://my/npm"
    )
    assert config.pypi_choice(custom) == "custom"
    assert config.github_choice(custom) == "custom"  # half-set pair is custom, not a preset
    assert config.npm_choice(custom) == "custom"
    off = config.MirrorConfig()
    assert config.pypi_choice(off) == "off"
    assert config.github_choice(off) == "off"
    assert config.npm_choice(off) == "off"


def test_github_base_recovers_a_custom_derivable_base() -> None:
    """github_base reverses a base-derived custom pair back to exactly its base (the value the
    single-URL github input round-trips), and returns "" for a pair no base expands to."""
    base = "https://my.mirror/gh"
    python_install, uv_binary = config.github_release_urls(base)
    m = config.compose(python_install=python_install, uv_binary=uv_binary)
    assert config.github_base(m) == base
    # A hand-edited pair that no single base expands to is not derivable.
    underivable = config.compose(python_install="https://x/py/", uv_binary="https://x/uv")
    assert config.github_base(underivable) == ""


def test_axis_choice_readers_are_blind_to_the_master_switch() -> None:
    # Three-state storage (on / paused / empty): a paused config keeps its URLs on disk and
    # the readers must still REPORT them. Visibility is the readers' job; whether an axis is
    # APPLIED is the master's (mirror_env / mirrors_line fold that in — never these readers).
    paused = config.MirrorConfig(
        enabled=False,
        pypi=config.PYPI_PRESETS["ustc"],
        npm=config.NPM_REGISTRY_MIRROR,
        python_install=config.PYTHON_INSTALL_MIRROR,
        uv_binary=config.UV_BINARY_MIRROR,
    )
    assert config.pypi_choice(paused) == "ustc"
    assert config.github_choice(paused) == "nju"
    assert config.npm_choice(paused) == "npmmirror"


def test_axis_display_helpers_exact() -> None:
    m = full_mirror()
    assert config.pypi_display(m) == "tsinghua"
    assert config.github_display(m) == "nju"
    assert config.npm_display(m) == "npmmirror"
    custom = config.compose(
        pypi="https://my/simple",
        python_install="https://my/py/",
        uv_binary="https://my/uv",
        npm="https://my/npm",
    )
    assert config.pypi_display(custom) == "https://my/simple"
    # An underivable, hand-edited github pair joins with " + " — never the " · " axes_summary
    # separator, which must stay unambiguous.
    assert config.github_display(custom) == "https://my/py/ + https://my/uv"
    assert config.npm_display(custom) == "https://my/npm"
    # A half-set github pair shows the live half and marks the blank half off.
    assert config.github_display(config.compose(python_install="https://my/py/")) == (
        "https://my/py/ + off"
    )
    assert config.github_display(config.compose(uv_binary="https://my/uv")) == (
        "off + https://my/uv"
    )
    off = config.MirrorConfig()
    assert config.pypi_display(off) == "off"
    assert config.github_display(off) == "off"
    assert config.npm_display(off) == "off"


def test_axes_summary_exact_strings() -> None:
    assert config.axes_summary(full_mirror()) == "pypi=tsinghua · github=nju · npm=npmmirror"
    assert config.axes_summary(config.MirrorConfig()) == "off"
    npm_only = config.compose(npm=config.NPM_PRESETS["npmmirror"])
    assert config.axes_summary(npm_only) == "pypi=off · github=off · npm=npmmirror"


def test_mirrors_line_three_states_exact() -> None:
    """F4: the one-line status (doctor, TUI health) tells the three storage states apart —
    empty off, on, and paused (off but URLs kept) — instead of collapsing paused into off."""
    assert config.mirrors_line(config.MirrorConfig()) == "Mirrors: off"
    assert config.mirrors_line(full_mirror()) == (
        "Mirrors: pypi=tsinghua · github=nju · npm=npmmirror"
    )
    paused = config.MirrorConfig(
        enabled=False,
        pypi=config.PYPI_PRESETS["tsinghua"],
        python_install=config.PYTHON_INSTALL_MIRROR,
        uv_binary=config.UV_BINARY_MIRROR,
        npm=config.NPM_REGISTRY_MIRROR,
    )
    assert config.mirrors_line(paused) == (
        "Mirrors: off (saved: pypi=tsinghua · github=nju · npm=npmmirror)"
    )


def test_update_mirror_axes_fresh_url_auto_enables() -> None:
    # Fresh (off, nothing saved): a first URL turns the master on — one-command setup.
    saved = config.update_mirror_axes(pypi=config.PYPI_PRESETS["tsinghua"])
    assert saved.enabled
    assert saved.pypi == config.PYPI_PRESETS["tsinghua"]
    assert config.load_mirror().enabled  # and it persisted


def test_update_mirror_axes_off_on_empty_stays_off() -> None:
    # Off applied to an empty config is a no-op that must NOT flip the master on.
    saved = config.update_mirror_axes(pypi="")
    assert not saved.enabled


def test_update_mirror_axes_enabled_stays_on_while_a_url_remains() -> None:
    config.save_mirror(full_mirror())
    saved = config.update_mirror_axes(pypi="")  # drop one of several
    assert saved.enabled
    assert saved.pypi == ""
    assert saved.npm == config.NPM_REGISTRY_MIRROR  # the others survive


def test_update_mirror_axes_clearing_the_last_url_disables() -> None:
    config.save_mirror(config.compose(npm=config.NPM_REGISTRY_MIRROR))
    saved = config.update_mirror_axes(npm="")
    assert not saved.enabled
    assert saved.npm == ""


def test_update_mirror_axes_paused_stays_paused_and_preserves_others() -> None:
    # Paused (off with URLs saved): a write keeps the master off — flipping it would
    # resurrect every other saved axis behind the user's back.
    config.save_mirror(full_mirror())
    config.disable()
    saved = config.update_mirror_axes(npm="https://new/npm")
    assert not saved.enabled  # still paused, not silently resurrected
    assert saved.npm == "https://new/npm"  # the asked-for change landed
    assert saved.pypi == config.PYPI_PRESETS["tsinghua"]  # untouched axis preserved


def test_update_mirror_axes_none_leaves_axes_untouched() -> None:
    config.save_mirror(full_mirror())
    saved = config.update_mirror_axes(npm="https://new/npm")  # only npm passed
    assert saved.pypi == config.PYPI_PRESETS["tsinghua"]
    assert saved.python_install == config.PYTHON_INSTALL_MIRROR
    assert saved.uv_binary == config.UV_BINARY_MIRROR


@pytest.mark.parametrize("axis", ["pypi", "python_install", "uv_binary", "npm"])
def test_enable_works_for_each_single_axis(axis: str) -> None:
    # Any ONE saved URL is enough to re-enable — each guard operand stands alone (an
    # or-chain, never and), because each axis is independently meaningful.
    config.save_mirror(config.compose(**{axis: "https://x"}))
    config.disable()
    assert config.enable() is True
    assert config.load_mirror().enabled


def test_enable_restores_saved_urls() -> None:
    config.save_mirror(full_mirror())
    config.disable()
    assert config.enable() is True
    m = config.load_mirror()
    assert m.enabled
    assert m.pypi == config.PYPI_PRESETS["tsinghua"]


def test_enable_refuses_when_nothing_saved() -> None:
    assert config.enable() is False
    assert not config.load_mirror().enabled


def test_save_mirror_preserves_other_keys() -> None:
    config.save_config({"language": "zh-CN"})
    config.save_mirror(config.compose(pypi=config.PYPI_PRESETS["ustc"]))
    doc = config.load_config()
    assert doc["language"] == "zh-CN"  # not clobbered by the mirror write
    assert doc["mirror"]["pypi"] == config.PYPI_PRESETS["ustc"]


def test_mirror_env_overlays_all_vectors() -> None:
    config.save_mirror(full_mirror())
    env = config.mirror_env({})
    assert env["UV_DEFAULT_INDEX"] == config.PYPI_PRESETS["tsinghua"]
    assert env["UV_PYTHON_INSTALL_MIRROR"] == config.PYTHON_INSTALL_MIRROR
    assert config.uv_binary_base() == config.UV_BINARY_MIRROR


@pytest.mark.parametrize("index_var", config._INDEX_ENV)
def test_mirror_env_defers_to_user_index(index_var: str) -> None:
    config.save_mirror(full_mirror())
    env = config.mirror_env({index_var: "https://mine/simple"})
    assert "UV_DEFAULT_INDEX" not in env  # the user's index wins
    assert "UV_PYTHON_INSTALL_MIRROR" in env  # the untouched vector is still injected


def test_mirror_env_defers_to_user_python_mirror() -> None:
    config.save_mirror(full_mirror())
    env = config.mirror_env({"UV_PYTHON_INSTALL_MIRROR": "https://mine/py/"})
    assert "UV_PYTHON_INSTALL_MIRROR" not in env
    assert "UV_DEFAULT_INDEX" in env


# --- (a) additive index vars must NOT trigger the defer (they don't replace uv's default index) ---


def test_mirror_env_does_not_defer_on_extra_index_url() -> None:
    config.save_mirror(full_mirror())
    env = config.mirror_env({"UV_EXTRA_INDEX_URL": "https://x"})
    # UV_EXTRA_INDEX_URL is additive, so the blocked default index is still live -> skit must inject.
    assert env["UV_DEFAULT_INDEX"] == config.PYPI_PRESETS["tsinghua"]


def test_mirror_env_does_not_defer_on_uv_index() -> None:
    config.save_mirror(full_mirror())
    env = config.mirror_env({"UV_INDEX": "https://x"})
    # UV_INDEX is additive too (F1: dropped from _INDEX_ENV), so injection must still happen.
    assert env["UV_DEFAULT_INDEX"] == config.PYPI_PRESETS["tsinghua"]


# --- (b) an empty-string user var means "unset": it must NOT suppress the mirror ---


def test_mirror_env_injects_when_index_env_blank() -> None:
    config.save_mirror(full_mirror())
    env = config.mirror_env({"UV_INDEX_URL": ""})
    assert env["UV_DEFAULT_INDEX"] == config.PYPI_PRESETS["tsinghua"]


def test_mirror_env_injects_when_python_mirror_blank() -> None:
    config.save_mirror(full_mirror())
    env = config.mirror_env({"UV_PYTHON_INSTALL_MIRROR": ""})
    assert env["UV_PYTHON_INSTALL_MIRROR"] == config.PYTHON_INSTALL_MIRROR


def test_disable_keeps_urls_but_turns_off() -> None:
    config.save_mirror(full_mirror())
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


def test_nju_preset_uv_binary_stays_https() -> None:
    # Sanity: the NJU github-release preset is https and must survive the https enforcement.
    config.save_mirror(full_mirror())
    assert config.load_mirror().uv_binary == config.UV_BINARY_MIRROR
    assert config.UV_BINARY_MIRROR.startswith("https://")


def test_load_config_tolerates_corrupt_toml(cfg_dir: Path) -> None:
    (cfg_dir / "config.toml").write_text("this is = = not [valid toml", encoding="utf-8")
    assert config.load_config() == {}


# --- corrupt config.toml must never be silently wiped by the next read-modify-write save ---


def test_save_editor_backs_up_corrupt_config_instead_of_wiping_it(
    cfg_dir: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    corrupt = 'language = "zh-CN"\n[mirror]\nenabled = true\npypi = "https://tsinghua"\nthis is = = not valid toml'
    (cfg_dir / "config.toml").write_text(corrupt, encoding="utf-8")
    config.save_editor("vim")
    # The just-requested change still takes effect...
    assert config.load_editor() == "vim"
    # ...but the corrupt original is preserved verbatim in a backup rather than vanishing.
    backup = cfg_dir / "config.toml.bak"
    assert backup.is_file()
    assert backup.read_text(encoding="utf-8") == corrupt
    # ...and the user is told on stderr, so the data loss isn't silent.
    err = capsys.readouterr().err
    assert "config.toml" in err
    assert "config.toml.bak" in err


def test_save_mirror_backs_up_corrupt_config_instead_of_wiping_it(cfg_dir: Path) -> None:
    corrupt = 'language = "zh-CN"\nthis is = = not valid toml'
    (cfg_dir / "config.toml").write_text(corrupt, encoding="utf-8")
    config.save_mirror(config.compose(pypi=config.PYPI_PRESETS["aliyun"]))
    assert config.load_mirror().pypi == config.PYPI_PRESETS["aliyun"]
    backup = cfg_dir / "config.toml.bak"
    assert backup.is_file()
    assert backup.read_text(encoding="utf-8") == corrupt


def test_save_editor_warns_when_corrupt_config_cannot_even_be_backed_up(
    cfg_dir: Path, capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    # Double failure (corrupt file + backup itself fails, e.g. a read-only config dir): the save
    # must still not crash, and must still tell the user on stderr that data may be lost.
    (cfg_dir / "config.toml").write_text("this is = = not valid toml", encoding="utf-8")

    def boom(*_a: object, **_k: object) -> None:
        raise OSError("disk full")

    monkeypatch.setattr(atomic.shutil, "copy2", boom)
    config.save_editor("vim")
    assert config.load_editor() == "vim"
    assert not (cfg_dir / "config.toml.bak").exists()
    err = capsys.readouterr().err
    assert "config.toml" in err


def test_save_editor_still_preserves_other_keys_when_config_is_valid(cfg_dir: Path) -> None:
    # Sanity: the fix must not regress the ordinary (non-corrupt) preserve-other-keys path.
    config.save_config({"language": "zh-CN"})
    config.save_editor("code --wait")
    doc = config.load_config()
    assert doc["language"] == "zh-CN"
    assert doc["editor"] == "code --wait"
    assert not (cfg_dir / "config.toml.bak").exists()


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


# --------------------------------------------------------------------------
# shell.bash_path (Windows escape hatch) + js.runner round-trips
# --------------------------------------------------------------------------


def test_bash_path_defaults_to_empty() -> None:
    assert config.load_bash_path() == ""


def test_bash_path_round_trip(tmp_path: Path) -> None:
    bash = tmp_path / "bash"
    bash.write_text("", encoding="utf-8")
    config.save_bash_path(str(bash))
    assert config.load_bash_path() == str(bash)


def test_bash_path_strips_and_clears() -> None:
    config.save_bash_path("  /opt/bash  ")
    assert config.load_bash_path() == "/opt/bash"  # stripped on save
    config.save_bash_path("")
    assert config.load_bash_path() == ""  # empty clears the key
    assert "shell" not in config.load_config()  # and drops the now-empty section


def test_bash_path_garbage_normalizes_to_empty() -> None:
    config.save_config({"shell": {"bash_path": 123}})  # not a string
    assert config.load_bash_path() == ""


def test_bash_path_garbage_section_normalizes_to_empty() -> None:
    config.save_config({"shell": "not-a-table"})  # section isn't a dict
    assert config.load_bash_path() == ""


def test_bash_path_save_preserves_other_keys() -> None:
    config.save_config({"language": "zh-CN"})
    config.save_bash_path("/opt/bash")
    doc = config.load_config()
    assert doc["language"] == "zh-CN"  # untouched
    assert doc["shell"]["bash_path"] == "/opt/bash"


def test_bash_path_clear_preserves_other_shell_keys() -> None:
    config.save_config({"shell": {"bash_path": "/x", "other": "keep"}})
    config.save_bash_path("")
    doc = config.load_config()
    assert doc["shell"] == {"other": "keep"}  # only bash_path removed; section stays


def test_js_runner_defaults_to_empty() -> None:
    assert config.load_js_runner() == ""


@pytest.mark.parametrize("name", config.JS_RUNNERS)
def test_js_runner_round_trip(name: str) -> None:
    config.save_js_runner(name)
    assert config.load_js_runner() == name


def test_js_runner_unknown_value_normalizes_to_empty() -> None:
    config.save_config({"js": {"runner": "carrier-pigeon"}})
    assert config.load_js_runner() == ""  # a hand-edited bad value must not poison runs


def test_js_runner_garbage_section_normalizes_to_empty() -> None:
    config.save_config({"js": ["not", "a", "table"]})
    assert config.load_js_runner() == ""


def test_js_runner_clears_and_drops_section() -> None:
    config.save_js_runner("deno")
    config.save_js_runner("")
    assert config.load_js_runner() == ""
    assert "js" not in config.load_config()


def test_js_runner_save_preserves_other_keys() -> None:
    config.save_config({"language": "en"})
    config.save_js_runner("bun")
    doc = config.load_config()
    assert doc["language"] == "en"
    assert doc["js"]["runner"] == "bun"
