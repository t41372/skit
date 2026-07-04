"""Launcher command assembly and workdir policy tests."""

from __future__ import annotations

from pathlib import Path

import pytest


@pytest.fixture(autouse=True)
def isolated_dirs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    return tmp_path


@pytest.fixture
def py_entry(tmp_path: Path):
    from skit import store

    p = tmp_path / "s.py"
    p.write_text("print('ok')\n", encoding="utf-8")
    return store.add_python(p)


def test_python_command_uses_uv_run_script(py_entry, monkeypatch):
    from skit import launcher

    monkeypatch.setattr(launcher, "find_uv", lambda: "/fake/uv")
    cmd = launcher.build_command(py_entry, ["--x", "1"])
    # C2: --script must be passed explicitly
    assert cmd[:3] == ["/fake/uv", "run", "--script"]
    assert cmd[3].endswith("script.py")
    assert cmd[-2:] == ["--x", "1"]


def test_python_without_uv_auto_downloads(py_entry, monkeypatch):
    """When uv is missing, fall back to the automatic download (A9); only raise LaunchError if
    the download also fails."""
    from skit import launcher, uvman

    monkeypatch.setattr(launcher, "find_uv", lambda: None)
    monkeypatch.setattr(uvman, "ensure_uv_downloaded", lambda **kw: "/downloaded/uv")
    cmd = launcher.build_command(py_entry)
    assert cmd[0] == "/downloaded/uv"


def test_python_uv_download_failure_raises(py_entry, monkeypatch):
    from skit import launcher, uvman

    def boom(**kw):
        raise uvman.UvDownloadError("network down")

    monkeypatch.setattr(launcher, "find_uv", lambda: None)
    monkeypatch.setattr(uvman, "ensure_uv_downloaded", boom)
    with pytest.raises(launcher.LaunchError, match="uv"):
        launcher.build_command(py_entry)


def test_command_template_appends_extra_args():
    from skit import launcher, store

    entry = store.add_command("echo hello", name="e")
    cmd = launcher.build_command(entry, ["world"])
    assert isinstance(cmd, str)
    assert cmd.startswith("echo hello")
    assert "world" in cmd


def test_workdir_origin_is_source_parent(py_entry, tmp_path):
    from skit.launcher import _resolve_workdir

    assert _resolve_workdir(py_entry, tmp_path / "elsewhere") == tmp_path


def test_workdir_store_and_invoke(py_entry, tmp_path):
    from skit.launcher import _resolve_workdir

    py_entry.meta.workdir = "store"
    assert _resolve_workdir(py_entry, tmp_path) == py_entry.dir
    py_entry.meta.workdir = "invoke"
    invoke = tmp_path / "cwd"
    assert _resolve_workdir(py_entry, invoke) == invoke


def test_run_entry_real_execution(py_entry):
    """Integration test: a real uv is present in the environment, so run for real."""
    import shutil

    from skit import launcher

    if shutil.which("uv") is None:
        pytest.skip("no uv in environment")
    code = launcher.run_entry(py_entry)
    assert code == 0


# ---------- find_uv: private-bin fallback ----------


def test_find_uv_private_bin_fallback(tmp_path, monkeypatch):
    """When uv is absent from PATH, find_uv should find the skit-private binary."""
    from skit import launcher

    monkeypatch.setattr("shutil.which", lambda _name: None)
    monkeypatch.setattr("skit.launcher.private_bin_dir", lambda: tmp_path / "bin")
    (tmp_path / "bin").mkdir()
    (tmp_path / "bin" / "uv").touch()
    assert launcher.find_uv() == str(tmp_path / "bin" / "uv")


def test_find_uv_returns_none_when_absent(tmp_path, monkeypatch):
    """When uv is in neither PATH nor the private bin, find_uv returns None."""
    from skit import launcher

    monkeypatch.setattr("shutil.which", lambda _name: None)
    monkeypatch.setattr("skit.launcher.private_bin_dir", lambda: tmp_path / "empty")
    assert launcher.find_uv() is None


# ---------- _resolve_workdir: source=None and absolute policy ----------


def test_workdir_origin_no_source_falls_back_to_cwd(py_entry, tmp_path):
    from skit.launcher import _resolve_workdir

    py_entry.meta.workdir = "origin"
    py_entry.meta.source = ""  # type: ignore[assignment]
    cwd = tmp_path / "cwd"
    assert _resolve_workdir(py_entry, cwd) == cwd


def test_workdir_absolute_path_used_directly(py_entry, tmp_path):
    from skit.launcher import _resolve_workdir

    custom = str(tmp_path / "custom")
    py_entry.meta.workdir = custom
    assert _resolve_workdir(py_entry, tmp_path) == Path(custom)


# ---------- _build_python: --with / --python flags ----------


def test_python_with_deps_and_python_version(py_entry, monkeypatch):
    from skit import launcher

    monkeypatch.setattr(launcher, "find_uv", lambda: "/uv")
    py_entry.meta.requires_python = ">=3.11"
    py_entry.meta.dependencies = ["requests", "rich"]
    cmd = launcher.build_command(py_entry)
    assert "--python" in cmd
    assert ">=3.11" in cmd
    assert cmd.count("--with") == 2


# ---------- _build_exe: source missing ----------


def test_exe_missing_source_raises(tmp_path, monkeypatch):
    from skit import launcher, store

    exe = tmp_path / "tool"
    exe.touch()
    entry = store.add_exe(exe)
    exe.unlink()  # simulate missing after add
    with pytest.raises(launcher.LaunchError, match="exe"):
        launcher.build_command(entry)


# ---------- build_command: unknown kind ----------


def test_build_command_unknown_kind_raises(py_entry):
    from skit import launcher

    py_entry.meta.kind = "unknown"  # type: ignore[assignment]
    with pytest.raises(launcher.LaunchError):
        launcher.build_command(py_entry)


# ---------- run_entry: missing workdir raises ----------


def test_run_entry_missing_workdir_raises(py_entry, monkeypatch):
    from skit import launcher

    monkeypatch.setattr(launcher, "find_uv", lambda: "/fake/uv")
    py_entry.meta.workdir = "/nonexistent/path/that/does/not/exist"
    with pytest.raises(launcher.LaunchError, match="/nonexistent/path"):
        launcher.run_entry(py_entry)


# ---------- run_entry: shell command execution ----------


def test_run_entry_command_entry(tmp_path, monkeypatch):
    import sys

    from skit import launcher, store

    if sys.platform == "win32":
        tmpl = "echo hello"
    else:
        tmpl = "echo hello"
    entry = store.add_command(tmpl, name="greet")
    code = launcher.run_entry(entry, invoke_cwd=tmp_path)
    assert code == 0


def test_run_entry_injects_mirror_env(py_entry, monkeypatch):
    from skit import config, launcher

    monkeypatch.delenv("UV_DEFAULT_INDEX", raising=False)
    monkeypatch.delenv("UV_PYTHON_INSTALL_MIRROR", raising=False)
    config.save_mirror(config.preset("tsinghua"))
    seen_env: dict[str, str] = {}

    class _Result:
        returncode = 0

    def _fake_run(_cmd, **kw):
        seen_env.update(kw.get("env", {}))
        return _Result()

    monkeypatch.setattr(launcher, "find_uv", lambda: "/fake/uv")
    monkeypatch.setattr(launcher.subprocess, "run", _fake_run)
    launcher.run_entry(py_entry)
    assert seen_env["UV_DEFAULT_INDEX"] == config.PYPI_PRESETS["tsinghua"]
    assert seen_env["UV_PYTHON_INSTALL_MIRROR"] == config.PYTHON_INSTALL_MIRROR


# ---------- (c) run_entry mirror env: disabled adds nothing; user's env wins end-to-end ----------


def _capture_run_env(monkeypatch, py_entry) -> dict[str, str]:
    """Run run_entry with subprocess.run stubbed and return the env handed to the child."""
    from skit import launcher

    seen_env: dict[str, str] = {}

    class _Result:
        returncode = 0

    def _fake_run(_cmd, **kw):
        seen_env.update(kw.get("env", {}))
        return _Result()

    monkeypatch.setattr(launcher, "find_uv", lambda: "/fake/uv")
    monkeypatch.setattr(launcher.subprocess, "run", _fake_run)
    launcher.run_entry(py_entry)
    return seen_env


def test_run_entry_no_mirror_env_when_disabled(py_entry, monkeypatch):
    """Mirror disabled (the default): the subprocess env gets no mirror variables injected."""
    monkeypatch.delenv("UV_DEFAULT_INDEX", raising=False)
    monkeypatch.delenv("UV_PYTHON_INSTALL_MIRROR", raising=False)
    seen_env = _capture_run_env(monkeypatch, py_entry)
    assert "UV_DEFAULT_INDEX" not in seen_env
    assert "UV_PYTHON_INSTALL_MIRROR" not in seen_env


def test_run_entry_keeps_user_index_when_mirror_enabled(py_entry, monkeypatch):
    """End-to-end defer: with the user's UV_DEFAULT_INDEX already in os.environ, run_entry keeps the
    user's value (mirror never clobbers it), while injecting the untouched python-install vector."""
    from skit import config

    config.save_mirror(config.preset("tsinghua"))
    monkeypatch.setenv("UV_DEFAULT_INDEX", "https://user/own/simple")
    monkeypatch.delenv("UV_PYTHON_INSTALL_MIRROR", raising=False)
    seen_env = _capture_run_env(monkeypatch, py_entry)
    assert seen_env["UV_DEFAULT_INDEX"] == "https://user/own/simple"
    assert seen_env["UV_PYTHON_INSTALL_MIRROR"] == config.PYTHON_INSTALL_MIRROR
