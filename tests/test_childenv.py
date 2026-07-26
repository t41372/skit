"""childenv: the frozen-binary loader-path scrub for child processes.

Unit-tests the scrub itself (unfrozen no-op, *_ORIG restore, bundle-entry filtering,
_PYI_* removal) and pins the seams that must consume it: launcher.run_entry and
editor.open_in_editor hand children a scrubbed environment, never raw os.environ.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

import pytest

from skit import childenv


@pytest.fixture
def bundle(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    """Simulate a PyInstaller-frozen process: sys.frozen + sys._MEIPASS at a real dir."""
    bundle_dir = tmp_path / "_MEI123"
    bundle_dir.mkdir()
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    monkeypatch.setattr(sys, "_MEIPASS", str(bundle_dir), raising=False)
    return bundle_dir


# ---------- unfrozen: a plain, independent copy ----------


def test_unfrozen_returns_plain_copy_of_base():
    base = {"LD_LIBRARY_PATH": "/opt/lib", "_PYI_ARCHIVE_FILE": "x", "KEEP": "1"}
    env = childenv.child_env(base)
    assert env == base
    env["KEEP"] = "mutated"
    assert base["KEEP"] == "1"  # a copy, not the caller's mapping


def test_unfrozen_defaults_to_os_environ(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_TEST_SENTINEL", "here")
    assert childenv.child_env()["SKIT_TEST_SENTINEL"] == "here"


def test_meipass_without_frozen_flag_is_untouched(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    # Some runtime setting _MEIPASS alone (or a stale attribute) must not trigger the scrub:
    # the frozen marker is required too.
    monkeypatch.setattr(sys, "_MEIPASS", str(tmp_path), raising=False)
    base = {"LD_LIBRARY_PATH": str(tmp_path)}
    assert childenv.child_env(base) == base


def test_frozen_without_meipass_is_untouched(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    monkeypatch.delattr(sys, "_MEIPASS", raising=False)
    base = {"LD_LIBRARY_PATH": "/anything", "_PYI_ARCHIVE_FILE": "x"}
    assert childenv.child_env(base) == base


# ---------- frozen: *_ORIG restore ----------


@pytest.mark.parametrize("var", ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH"])
def test_frozen_restores_loader_var_from_orig(var: str, bundle: Path):
    base = {var: str(bundle), f"{var}_ORIG": "/the/original"}
    env = childenv.child_env(base)
    assert env[var] == "/the/original"
    assert f"{var}_ORIG" not in env


def test_frozen_orig_restores_even_a_user_looking_value(bundle: Path):
    # When the bootloader stashed an original, that stash IS the pre-launch truth — restore
    # it without inspecting the current value.
    base = {"LD_LIBRARY_PATH": "/user/lib", "LD_LIBRARY_PATH_ORIG": "/orig/lib"}
    assert childenv.child_env(base)["LD_LIBRARY_PATH"] == "/orig/lib"


# ---------- frozen: no *_ORIG — filter bundle entries ----------


def test_frozen_drops_var_that_is_only_the_bundle(bundle: Path):
    env = childenv.child_env({"LD_LIBRARY_PATH": str(bundle)})
    assert "LD_LIBRARY_PATH" not in env


def test_frozen_keeps_non_bundle_entries(bundle: Path):
    value = os.pathsep.join([str(bundle), "/usr/lib", str(bundle / "sub")])
    env = childenv.child_env({"LD_LIBRARY_PATH": value})
    assert env["LD_LIBRARY_PATH"] == "/usr/lib"


def test_frozen_drops_empty_entries_alongside_bundle_ones(bundle: Path):
    value = os.pathsep + "/usr/lib" + os.pathsep
    env = childenv.child_env({"LD_LIBRARY_PATH": value})
    assert env["LD_LIBRARY_PATH"] == "/usr/lib"


def test_frozen_drops_var_left_with_only_empty_entries(bundle: Path):
    env = childenv.child_env({"LD_LIBRARY_PATH": os.pathsep})
    assert "LD_LIBRARY_PATH" not in env


def test_frozen_leaves_absent_loader_vars_absent(bundle: Path):
    env = childenv.child_env({"HOME": "/home/u"})
    assert env == {"HOME": "/home/u"}


# ---------- frozen: PATH is filtered, never ORIG-restored ----------


def test_frozen_filters_bundle_from_path(bundle: Path):
    value = os.pathsep.join(["/usr/bin", str(bundle)])
    env = childenv.child_env({"PATH": value})
    assert env["PATH"] == "/usr/bin"


def test_frozen_drops_path_that_is_only_the_bundle(bundle: Path):
    env = childenv.child_env({"PATH": str(bundle)})
    assert "PATH" not in env


def test_frozen_path_without_bundle_entries_is_untouched(bundle: Path):
    # A trailing empty entry is the user's own POSIX cwd-on-PATH choice; a PATH that
    # contains no bundle entry is none of the scrub's business and must round-trip
    # byte-identical (pip-installed and frozen skit behave the same).
    value = "/usr/bin" + os.pathsep
    env = childenv.child_env({"PATH": value})
    assert env["PATH"] == value


def test_frozen_path_keeps_empty_entries_when_filtering(bundle: Path):
    value = os.pathsep.join(["/usr/bin", str(bundle), ""])
    env = childenv.child_env({"PATH": value})
    assert env["PATH"] == os.pathsep.join(["/usr/bin", ""])


def test_frozen_path_orig_is_not_special(bundle: Path):
    # PATH has no bootloader stash contract; a PATH_ORIG variable (whatever set it) must be
    # passed through untouched, not consumed.
    base = {"PATH": "/usr/bin", "PATH_ORIG": "/stale"}
    env = childenv.child_env(base)
    assert env["PATH"] == "/usr/bin"
    assert env["PATH_ORIG"] == "/stale"


# ---------- frozen: bootloader bookkeeping ----------


def test_frozen_drops_pyi_internal_vars(bundle: Path):
    base = {"_PYI_ARCHIVE_FILE": "/x", "_PYI_PARENT_PROCESS_LEVEL": "1", "PYI_KEEP": "y"}
    env = childenv.child_env(base)
    assert "_PYI_ARCHIVE_FILE" not in env
    assert "_PYI_PARENT_PROCESS_LEVEL" not in env
    assert env["PYI_KEEP"] == "y"  # only the "_PYI_" prefix is the bootloader's namespace


# ---------- delivery values: {env:X} must read the child's environment ----------


def test_env_token_expands_the_scrubbed_environment(
    bundle: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    from skit import tokens

    monkeypatch.setenv("LD_LIBRARY_PATH", str(bundle))
    monkeypatch.setenv("LD_LIBRARY_PATH_ORIG", "/real/lib")
    assert tokens.expand("{env:LD_LIBRARY_PATH}", cwd=tmp_path) == "/real/lib"


def test_env_token_rejects_bootloader_bookkeeping(
    bundle: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    # Unfrozen skit would raise "isn't set" for a variable that doesn't exist outside
    # the bootloader; frozen skit must behave identically, not leak _PYI_* internals.
    from skit import tokens

    monkeypatch.setenv("_PYI_ARCHIVE_FILE", "/x")
    with pytest.raises(tokens.TokenError):
        tokens.expand("{env:_PYI_ARCHIVE_FILE}", cwd=tmp_path)


def test_assemble_env_tokens_use_scrubbed_env(
    bundle: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    from skit import flows

    monkeypatch.setenv("LD_LIBRARY_PATH", str(bundle))
    monkeypatch.setenv("LD_LIBRARY_PATH_ORIG", "/real/lib")
    plan = flows.FormPlan(
        source="inject", fields=[flows.FormField(key="v", label="v", source="inject")]
    )
    out = flows.assemble(plan, {"v": "{env:LD_LIBRARY_PATH}"}, [], cwd=tmp_path)
    assert out.inject_values["v"] == "/real/lib"


# ---------- seams: children must receive the scrubbed environment ----------


def test_run_entry_child_env_is_scrubbed(bundle: Path, monkeypatch: pytest.MonkeyPatch):
    from skit import launcher, store

    monkeypatch.setenv("LD_LIBRARY_PATH", str(bundle))
    monkeypatch.setenv("LD_LIBRARY_PATH_ORIG", "/real/lib")
    seen_env: dict[str, str] = {}

    class _Result:
        returncode = 0

    def _fake_run(_cmd, **kw):
        seen_env.update(kw["env"])
        return _Result()

    monkeypatch.setattr(launcher.subprocess, "run", _fake_run)
    entry = store.add_command("echo hello", name="greet")
    assert launcher.run_entry(entry) == 0
    assert seen_env["LD_LIBRARY_PATH"] == "/real/lib"
    assert "LD_LIBRARY_PATH_ORIG" not in seen_env


def test_shell_gate_child_env_is_scrubbed(
    bundle: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    from skit.langs.shell import inject as shell_inject

    monkeypatch.setenv("LD_LIBRARY_PATH", str(bundle))
    monkeypatch.delenv("LD_LIBRARY_PATH_ORIG", raising=False)
    seen: dict[str, object] = {}

    class _Proc:
        returncode = 0
        stderr = b""

    def _fake_run(argv, **kw):
        seen.update(kw)
        return _Proc()

    monkeypatch.setattr(shell_inject.subprocess, "run", _fake_run)
    monkeypatch.setattr(shell_inject.shutil, "which", lambda _name: "/bin/bash")
    shell_inject._gate_interpreter("bash", tmp_path / "x.sh")
    env = seen["env"]
    assert isinstance(env, dict)
    assert "LD_LIBRARY_PATH" not in env


def test_powershell_probe_env_is_scrubbed(
    bundle: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    from skit.langs.powershell import cli_reader

    monkeypatch.setenv("LD_LIBRARY_PATH", str(bundle))
    monkeypatch.delenv("LD_LIBRARY_PATH_ORIG", raising=False)
    seen_env: dict[str, str] = {}

    class _Proc:
        returncode = 0
        stdout = b'{"status":"no-params"}'

    def _fake_run(argv, **kw):
        seen_env.update(kw["env"])
        return _Proc()

    monkeypatch.setattr(cli_reader.subprocess, "run", _fake_run)
    target = tmp_path / "x.ps1"
    assert cli_reader._extract("pwsh", target) is None  # no-params reads as None
    # The probe's own variable must ride on top of the SCRUBBED environment, not
    # alongside the bootloader's loader path.
    assert seen_env["SKIT_PS_TARGET"] == str(target)
    assert "LD_LIBRARY_PATH" not in seen_env


def test_open_in_editor_child_env_is_scrubbed(
    bundle: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    from skit import editor

    monkeypatch.setenv("LD_LIBRARY_PATH", str(bundle))
    monkeypatch.delenv("LD_LIBRARY_PATH_ORIG", raising=False)
    monkeypatch.setenv("EDITOR", "true")
    monkeypatch.setattr("skit.config.load_editor", lambda: "")
    monkeypatch.delenv("VISUAL", raising=False)
    seen: dict[str, object] = {}

    class _Result:
        returncode = 0

    def _fake_run(argv, **kw):
        seen["env"] = kw["env"]
        return _Result()

    monkeypatch.setattr(editor.subprocess, "run", _fake_run)
    assert editor.open_in_editor(tmp_path / "f.sh") == 0
    env = seen["env"]
    assert isinstance(env, dict)
    assert "LD_LIBRARY_PATH" not in env
