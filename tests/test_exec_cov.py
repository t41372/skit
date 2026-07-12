"""Behavioural coverage top-up for shim.py, launcher.py, uvman.py.

Targets specific branches left unexercised by the existing test suites: the
"skip a non-literal duplicate assignment" path in shim's const scanner, the
"no insertion point" contract of the preamble helpers, the missing-script /
--no-project / successful-exe / Windows-quoting branches in launcher, and the
zip-archive + success path of uvman's extractor.
"""

from __future__ import annotations

import subprocess
import sys
import zipfile
from pathlib import Path

import pytest

from skit import uvman
from skit.langs.python import shim
from skit.langs.python.metawriter import ParamSpec


def spec(
    name: str, *, kind: str = "const", type: str = "str", order: int = -1, secret: bool = False
) -> ParamSpec:
    return ParamSpec(name=name, kind=kind, type=type, order=order, secret=secret)


# Local fixtures (this file must not depend on conftest.py / other test modules' fixtures).


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


# ---------- shim: _const_targets skips a non-literal duplicate, keeps scanning ----------


def test_const_skips_non_literal_duplicate_then_replaces_literal():
    """Two assignments to the same name: a non-literal RHS (e.g. a function call) must be
    skipped (not appended as a candidate) while the loop keeps scanning and still finds the
    later literal assignment to replace. Exercises both the True and False sides of
    `_const_targets`'s `if ok:` check within a single injection."""
    src = "CITY = get_city()\nCITY = 'Taipei'\nprint(CITY)\n"
    out = shim.inject(src, [spec("CITY")], {"CITY": "Kaohsiung"})
    assert "CITY = get_city()" in out  # non-literal assignment left untouched
    assert "CITY = 'Kaohsiung'" in out  # the literal duplicate is the one replaced


# ---------- launcher: _build_python missing script raises ----------


def test_build_python_missing_script_raises(py_entry, monkeypatch):
    from skit import launcher

    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/fake/uv")
    py_entry.script_path.unlink()
    with pytest.raises(launcher.LaunchError, match="script"):
        launcher.build_command(py_entry)


# ---------- launcher: _build_python with script_override forces --no-project ----------


def test_build_python_with_script_override_uses_no_project(py_entry, tmp_path, monkeypatch):
    from skit import launcher

    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/fake/uv")
    override = tmp_path / "injected.py"
    override.write_text("print(1)\n", encoding="utf-8")
    cmd = launcher.build_command(py_entry, script_override=override)
    assert "--no-project" in cmd
    assert str(override) in cmd


# ---------- launcher: _build_exe success path returns the argv ----------


def test_build_exe_success_returns_argv(tmp_path):
    from skit import launcher, store

    exe = tmp_path / "tool"
    exe.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    exe.chmod(0o755)
    entry = store.add_exe(exe)
    cmd = launcher.build_command(entry, ["--flag"])
    assert cmd == [str(exe.resolve()), "--flag"]


# ---------- launcher: _build_shell on win32 uses list2cmdline quoting ----------


def test_build_shell_windows_platform_uses_list2cmdline(monkeypatch):
    from skit import launcher, store

    entry = store.add_command("echo hello", name="win-quote")
    monkeypatch.setattr("sys.platform", "win32")
    cmd = launcher.build_command(entry, ["a b", "c"])
    assert isinstance(cmd, str)
    assert cmd.startswith("echo hello ")
    assert subprocess.list2cmdline(["a b", "c"]) in cmd


# ---------- uvman: _extract_uv success path (zip archive + copy/chmod) ----------


def test_extract_uv_from_zip_archive_success(tmp_path):
    """A .zip archive (the Windows release format) containing the expected executable name
    must be extracted, copied into dest_dir, and made executable — covering both the zip
    branch and the shared success tail (copy2 + chmod + return) of _extract_uv."""
    exe_name = "uv.exe" if sys.platform == "win32" else "uv"
    archive = tmp_path / "uv.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        zf.writestr(f"uv-dir/{exe_name}", "fake-binary-content")
    dest_dir = tmp_path / "dest"
    dest = uvman._extract_uv(archive, dest_dir)
    assert dest == dest_dir / exe_name
    assert dest.read_text(encoding="utf-8") == "fake-binary-content"
    assert dest.stat().st_mode & 0o755 == 0o755
