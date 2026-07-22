"""Mutation-kill tests for src/skit/langs/launch.py.

Each test pins an observable contract of the launch strategies through a real code path
(the pure helpers directly, and UvLaunch/RunnerLaunch/TemplateLaunch via their public
build/describe/preflight surface, mirroring tests/test_interpreters.py). Message-content
assertions force the English catalog so a mutated msgid (the XX-wrapped / re-cased
variants mutmut generates) diverges from the pinned source string.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import i18n, launcher, store
from skit.langs import launch
from skit.langs.base import ArgvLaunch, LaunchPayload, NotExecutableError
from skit.models import Entry, Mode, ScriptMeta


@pytest.fixture(autouse=True)
def _iso(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    # Pin the catalog to English so message assertions compare against the English source
    # msgid regardless of whatever locale a prior test left in the module-global state.
    i18n.init("en")


def _which_map(mapping: dict[str, str]):
    """A fake launch._which that resolves only the names in `mapping` (else None)."""
    return mapping.get


def _argv(payload: LaunchPayload) -> list[str]:
    assert isinstance(payload, ArgvLaunch)
    return payload.argv


def _entry(
    tmp_path: Path,
    kind: str,
    *,
    interpreter: str = "",
    mode: Mode = "copy",
    body: str = "console.log(1)\n",
    make_file: bool = True,
    source: str = "",
) -> Entry:
    d = tmp_path / "e"
    d.mkdir(exist_ok=True)
    meta = ScriptMeta(name="e", kind=kind, mode=mode, interpreter=interpreter, source=source)
    entry = Entry(slug="e", meta=meta, dir=d)
    if make_file:
        entry.script_path.write_text(body, encoding="utf-8")
    return entry


@pytest.fixture
def py_entry(tmp_path: Path):
    p = tmp_path / "s.py"
    p.write_text("print('ok')\n", encoding="utf-8")
    return store.add_python(p)


# ==========================================================================
# _check_exe_exists — the win32 x-bit-check guard + its message
# ==========================================================================


def test_check_exe_exists_skips_xbit_check_on_win32(tmp_path: Path, monkeypatch):
    # On win32 the execute-bit check is deliberately skipped (no POSIX +x concept), so a
    # non-executable file must pass. If the "win32" literal is mutated, the != guard becomes
    # true on win32 and the branch wrongly raises for the un-executable file.
    monkeypatch.setattr("sys.platform", "win32")
    f = tmp_path / "tool.bat"
    f.write_text("echo hi\n", encoding="utf-8")
    f.chmod(0o644)  # exists, regular file, no execute bit
    launch._check_exe_exists(str(f))  # must NOT raise on win32


def test_check_exe_exists_non_executable_message_is_the_english_source(tmp_path: Path, monkeypatch):
    monkeypatch.setattr("sys.platform", "linux")
    # os.access(X_OK) is faked False so the exec-bit refusal fires deterministically on every
    # platform (real Windows has no execute bit and returns True regardless of chmod, which would
    # make this branch unreachable there).
    monkeypatch.setattr(launch.os, "access", lambda _path, _mode: False)
    f = tmp_path / "tool"
    f.write_text("bytes\n", encoding="utf-8")
    with pytest.raises(NotExecutableError) as exc:
        launch._check_exe_exists(str(f))
    assert str(exc.value) == f"{f} exists but isn't executable (chmod +x it?)."


def test_check_exe_exists_refuses_a_directory_message_is_the_english_source(tmp_path: Path):
    # Fix B: a directory (a macOS .app bundle, a typo'd path) must be refused with a clean
    # NotExecutableError (exit 126) BEFORE the POSIX X_OK check — otherwise it reaches
    # subprocess.run and dies with a raw PermissionError traceback. The is_file() gate is
    # platform-independent, so this fires everywhere; the message names the offending path.
    d = tmp_path / "Bundle.app"
    d.mkdir()
    with pytest.raises(NotExecutableError) as exc:
        launch._check_exe_exists(str(d))
    assert str(exc.value) == f"{d} isn't a runnable file (it's a directory or special file)."


def test_check_exe_exists_regular_executable_file_passes(tmp_path: Path, monkeypatch):
    # The other side of Fix B's reorder: a regular file with the execute bit set clears the new
    # is_file() gate AND the X_OK check, so _check_exe_exists returns without raising. Pins the
    # X_OK-true branch so a mutant flipping `not os.access(...)` is caught (it would raise here).
    monkeypatch.setattr("sys.platform", "linux")
    monkeypatch.setattr(launch.os, "access", lambda _path, _mode: True)
    f = tmp_path / "tool"
    f.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    launch._check_exe_exists(str(f))  # must not raise


# ==========================================================================
# resolve_interpreter — shell-family tuple + refusal messages
# ==========================================================================


def test_resolve_zsh_on_win32_is_in_shell_family(monkeypatch):
    # zsh is a shell-family name: on win32 with nothing on PATH and no configured bash it must
    # reach the Git-for-Windows escape-hatch message, not the generic "isn't installed" one.
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.setattr("skit.config.load_bash_path", lambda: "")
    with pytest.raises(NotExecutableError) as exc:
        launch.resolve_interpreter("zsh")
    assert "Git for Windows" in str(exc.value)


def test_resolve_sh_on_win32_is_in_shell_family(monkeypatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.setattr("skit.config.load_bash_path", lambda: "")
    with pytest.raises(NotExecutableError) as exc:
        launch.resolve_interpreter("sh")
    assert "Git for Windows" in str(exc.value)


def test_resolve_bash_win32_unset_message_is_the_english_source(monkeypatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.setattr("skit.config.load_bash_path", lambda: "")
    with pytest.raises(NotExecutableError) as exc:
        launch.resolve_interpreter("bash")
    assert str(exc.value) == (
        "bash isn't available on this system. Install Git for Windows (its bash "
        "works) or WSL, or point skit at one with: skit config shell.bash_path <path>"
    )


def test_resolve_missing_posix_generic_message_is_the_english_source(monkeypatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "linux")
    with pytest.raises(NotExecutableError) as exc:
        launch.resolve_interpreter("ruby")
    assert str(exc.value) == "The interpreter ruby isn't installed (or isn't on PATH)."


# ==========================================================================
# RunnerLaunch._preferred_name — meta.interpreter wins via `or`
# ==========================================================================


def test_runner_describe_prefers_meta_interpreter_over_order(tmp_path: Path, monkeypatch):
    # describe's preferred name is `meta.interpreter or config or ORDER[0]`; an explicit
    # meta.interpreter must win outright. The `or`->`and` mutation would drop it back to the
    # ORDER default (deno).
    monkeypatch.setattr("skit.config.load_js_runner", lambda: "")
    entry = _entry(tmp_path, "js", interpreter="bun")
    described = launch.RunnerLaunch().describe(entry, [], None, None)
    assert described.startswith("bun ")


# ==========================================================================
# RunnerLaunch._resolve — the "no runtime" refusal message
# ==========================================================================


def test_runner_none_installed_message_is_the_english_source(tmp_path: Path, monkeypatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("skit.config.load_js_runner", lambda: "")
    entry = _entry(tmp_path, "js")
    with pytest.raises(NotExecutableError) as exc:
        launch.RunnerLaunch().build(entry, [], None, None)
    assert str(exc.value) == (
        "No JavaScript runtime found (looked for: deno, bun, node). "
        "Install deno, bun, or node — or pick one with: skit config js.runner <name>"
    )


# ==========================================================================
# RunnerLaunch.build / describe — the _INVOKE default and script/argv wiring
# ==========================================================================


def test_runner_build_unknown_runner_has_no_invoke_prefix(tmp_path: Path, monkeypatch):
    # A runner not in _INVOKE (a custom meta.interpreter) contributes no sub-command: the
    # `.get(name, ())` default must stay an empty tuple. Mutating it to None makes `*None`
    # a TypeError at build time.
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"tsx": "/usr/bin/tsx"}))
    entry = _entry(tmp_path, "js", interpreter="tsx")
    payload = launch.RunnerLaunch().build(entry, ["--flag"], None, None)
    assert _argv(payload) == ["/usr/bin/tsx", str(entry.script_path), "--flag"]


def test_runner_describe_unknown_runner_has_no_invoke_prefix(tmp_path: Path, monkeypatch):
    monkeypatch.setattr("skit.config.load_js_runner", lambda: "")
    entry = _entry(tmp_path, "js", interpreter="tsx")
    described = launch.RunnerLaunch().describe(entry, [], None, None)
    assert described.startswith("tsx ")
    assert str(entry.script_path) in described


def test_runner_describe_honors_script_override(tmp_path: Path, monkeypatch):
    # describe shows `script_override or entry.script_path`; when an override is given it is
    # what appears. Catches script->None, str(None), and the `or`->`and` inversion.
    monkeypatch.setattr("skit.config.load_js_runner", lambda: "")
    entry = _entry(tmp_path, "js")
    override = tmp_path / "override.js"
    described = launch.RunnerLaunch().describe(entry, [], None, override)
    assert str(override) in described
    assert str(entry.script_path) not in described


# ==========================================================================
# RunnerLaunch.preflight — installer demand only when the marker is stale
# ==========================================================================


def _js_deps_entry(tmp_path: Path) -> Entry:
    entry = _entry(tmp_path, "js", interpreter="bun", source="foo.mjs")
    entry.meta.dependencies = ["chalk"]
    return entry


def test_preflight_up_to_date_marker_demands_no_installer(tmp_path: Path, monkeypatch):
    # A node_modules marker that matches the (deps, runner, module_type) stamp means no install
    # is pending, so preflight must NOT demand the installer. Mutations that corrupt the runner
    # or module_type passed to needs_install make the stamp mismatch -> a spurious installer
    # demand -> a raise (which shutil.which=None guarantees).
    from skit.langs.javascript import deps as js_deps

    entry = _js_deps_entry(tmp_path)
    _installer, _manifest, stamp = js_deps._resolve_manifest(["chalk"], "bun", "module")
    node_modules = entry.dir / "node_modules"
    node_modules.mkdir(parents=True)
    (node_modules / ".skit-deps-ok").write_text(stamp, encoding="utf-8")

    monkeypatch.setattr("skit.langs.launch._which", _which_map({"bun": "/usr/bin/bun"}))
    monkeypatch.setattr("shutil.which", lambda _name: None)
    launch.RunnerLaunch().preflight(entry)  # marker current -> no installer demanded, no raise


def test_preflight_missing_installer_names_the_resolved_runner(tmp_path: Path, monkeypatch):
    # No node_modules yet -> an install is pending -> preflight demands the runner's installer.
    # The demand must name the RESOLVED runner (bun), not the npm fallback a None runner yields.
    entry = _js_deps_entry(tmp_path)
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"bun": "/usr/bin/bun"}))
    monkeypatch.setattr("shutil.which", lambda _name: None)
    with pytest.raises(NotExecutableError) as exc:
        launch.RunnerLaunch().preflight(entry)
    assert "bun" in str(exc.value)


# ==========================================================================
# TemplateLaunch.describe — extra args are appended
# ==========================================================================


def test_template_describe_appends_extra_args(tmp_path: Path):
    entry = store.add_command("echo hi", name="tmpl")
    assert launcher.describe_command(entry, ["world"]) == "echo hi world"


# ==========================================================================
# UvLaunch.describe — the transparency command line
# ==========================================================================


def test_uv_describe_exact_command_line(py_entry, monkeypatch):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: "/fake/uv")
    line = launcher.describe_command(py_entry, ["--x", "1"])
    assert line.split() == [
        "/fake/uv",
        "run",
        "--no-project",
        "--script",
        str(py_entry.script_path),
        "--x",
        "1",
    ]


def test_uv_describe_literal_uv_stands_in_when_absent(py_entry, monkeypatch):
    monkeypatch.setattr("skit.langs.launch.find_uv", lambda: None)
    line = launcher.describe_command(py_entry)
    assert line.split()[0] == "uv"  # the literal 'uv' fallback, not a re-cased/wrapped variant
