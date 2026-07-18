"""Tier-0 multi-language launch: interpreter resolution, the InterpreterLaunch /
RunnerLaunch strategies, shebang sniffing + kind inference, the `needs` preflight
contract, and the CLI surfaces the feature adds (--kind, deps --need, doctor/show
needs). Every assertion pins an observable contract; the mutation-grade cases patch
the one seam each strategy funnels PATH lookups through (`skit.langs.launch._which`)
so no test depends on what the host actually has installed.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, launcher, store
from skit.langs import launch
from skit.langs import registry as reg
from skit.langs.base import ArgvLaunch, LaunchPayload, NotExecutableError, TargetMissingError
from skit.models import Entry, Mode, ScriptMeta
from skit.params import ParamDecl

runner = CliRunner()


def _argv(payload: LaunchPayload) -> list[str]:
    """Narrow a built payload to its argv (every interpreted/runner build is ArgvLaunch —
    the assert both documents that and satisfies the exhaustive-union type checker)."""
    assert isinstance(payload, ArgvLaunch)
    return payload.argv


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")


def _which_map(mapping: dict[str, str]):
    """A fake _which resolving only the names in `mapping` (everything else is missing).
    dict.get IS the lookup: present name -> path, absent -> None."""
    return mapping.get


def _entry(
    tmp_path: Path,
    kind: str,
    *,
    interpreter: str = "",
    mode: Mode = "copy",
    body: str = "echo hi\n",
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


# ==========================================================================
# resolve_interpreter
# ==========================================================================


def test_resolve_interpreter_found_on_path(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"bash": "/usr/bin/bash"}))
    assert launch.resolve_interpreter("bash") == "/usr/bin/bash"


def test_resolve_interpreter_missing_posix_names_the_interpreter(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "linux")
    with pytest.raises(NotExecutableError) as exc:
        launch.resolve_interpreter("zsh")
    assert "zsh" in str(exc.value)


def test_resolve_bash_on_win32_uses_config_path_when_it_exists(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    bash = tmp_path / "bash.exe"
    bash.write_text("", encoding="utf-8")
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.setattr("skit.config.load_bash_path", lambda: str(bash))
    assert launch.resolve_interpreter("bash") == str(bash)


def test_resolve_bash_on_win32_configured_but_missing_falls_through(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.setattr("skit.config.load_bash_path", lambda: str(tmp_path / "gone.exe"))
    with pytest.raises(NotExecutableError) as exc:
        launch.resolve_interpreter("bash")
    assert "Git for Windows" in str(exc.value)


def test_resolve_bash_on_win32_unset_names_both_escape_hatches(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "win32")
    monkeypatch.setattr("skit.config.load_bash_path", lambda: "")
    with pytest.raises(NotExecutableError) as exc:
        launch.resolve_interpreter("bash")
    msg = str(exc.value)
    assert "Git for Windows" in msg
    assert "skit config shell.bash_path" in msg  # the message teaches a REAL command


def test_resolve_nonbash_on_win32_gets_generic_message(monkeypatch: pytest.MonkeyPatch):
    # Only bash/sh/zsh get the Windows escape-hatch path; a missing ruby gets the plain
    # "isn't installed" refusal (no config key to point at).
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "win32")
    with pytest.raises(NotExecutableError) as exc:
        launch.resolve_interpreter("ruby")
    msg = str(exc.value)
    assert "ruby" in msg
    assert "Git for Windows" not in msg


def test_which_seam_is_the_real_shutil_which():
    # The one PATH-lookup seam every strategy funnels through: a name that cannot exist
    # returns None (proves it delegates to shutil.which, not a stub).
    assert launch._which("skit-definitely-not-a-real-binary-zzz") is None


# ==========================================================================
# InterpreterLaunch
# ==========================================================================


def test_interpreter_launch_builds_argv(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"bash": "/bin/bash"}))
    entry = _entry(tmp_path, "shell")
    payload = launch.InterpreterLaunch("bash").build(entry, ["--fast"], None, None)
    assert _argv(payload) == ["/bin/bash", str(entry.script_path), "--fast"]


def test_interpreter_launch_meta_interpreter_beats_default(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"zsh": "/bin/zsh"}))
    entry = _entry(tmp_path, "shell", interpreter="zsh")  # #!/bin/zsh script kept its dialect
    payload = launch.InterpreterLaunch("bash").build(entry, [], None, None)
    assert _argv(payload)[0] == "/bin/zsh"


def test_interpreter_launch_prefix_placement(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"pwsh": "/usr/bin/pwsh"}))
    entry = _entry(tmp_path, "powershell", body="Write-Host hi\n")
    payload = launch.InterpreterLaunch("pwsh", prefix=("-File",)).build(entry, [], None, None)
    # -File sits between the interpreter and the script (PowerShell file semantics).
    assert _argv(payload) == ["/usr/bin/pwsh", "-File", str(entry.script_path)]


def test_interpreter_launch_describe_is_side_effect_free(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    def boom(_name: str) -> str:
        raise AssertionError("describe must not resolve the interpreter on PATH")

    monkeypatch.setattr("skit.langs.launch._which", boom)
    entry = _entry(tmp_path, "shell")
    described = launch.InterpreterLaunch("bash").describe(entry, ["-x"], None, None)
    assert described.startswith("bash ")  # the bare name stands in, no PATH lookup
    assert "-x" in described


def test_interpreter_launch_target_is_script_path(tmp_path: Path):
    entry = _entry(tmp_path, "shell")
    assert launch.InterpreterLaunch("bash").target(entry) == entry.script_path


def test_interpreter_launch_preflight_missing_interpreter(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    monkeypatch.setattr("sys.platform", "linux")
    entry = _entry(tmp_path, "shell")
    with pytest.raises(NotExecutableError):
        launch.InterpreterLaunch("bash").preflight(entry)


def test_interpreter_launch_preflight_ok(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"bash": "/bin/bash"}))
    entry = _entry(tmp_path, "shell")
    launch.InterpreterLaunch("bash").preflight(entry)  # script exists + interpreter resolves


def test_interpreter_launch_missing_script_raises_before_resolution(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    def boom(_name: str) -> str:
        raise AssertionError("interpreter must not be resolved when the script is missing")

    monkeypatch.setattr("skit.langs.launch._which", boom)
    entry = _entry(tmp_path, "shell", make_file=False)
    with pytest.raises(TargetMissingError):
        launch.InterpreterLaunch("bash").build(entry, [], None, None)


# ==========================================================================
# RunnerLaunch
# ==========================================================================


def test_runner_detection_order_prefers_deno(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(
        "skit.langs.launch._which",
        _which_map({"deno": "/d", "bun": "/b", "node": "/n"}),
    )
    entry = _entry(tmp_path, "js", body="console.log(1)\n")
    payload = launch.RunnerLaunch().build(entry, [], None, None)
    assert _argv(payload) == ["/d", "run", "--allow-all", str(entry.script_path)]


def test_runner_falls_to_bun_then_node(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    entry = _entry(tmp_path, "js", body="console.log(1)\n")
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"bun": "/b", "node": "/n"}))
    assert _argv(launch.RunnerLaunch().build(entry, [], None, None)) == [
        "/b",
        "run",
        str(entry.script_path),
    ]
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"node": "/n"}))
    # node takes no "run" subcommand.
    assert _argv(launch.RunnerLaunch().build(entry, [], None, None)) == [
        "/n",
        str(entry.script_path),
    ]


def test_runner_meta_interpreter_override(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(
        "skit.langs.launch._which", _which_map({"deno": "/d", "bun": "/b", "node": "/n"})
    )
    entry = _entry(tmp_path, "js", interpreter="bun", body="console.log(1)\n")
    assert _argv(launch.RunnerLaunch().build(entry, [], None, None))[0] == "/b"


def test_runner_config_override(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({"deno": "/d", "node": "/n"}))
    monkeypatch.setattr("skit.config.load_js_runner", lambda: "node")
    entry = _entry(tmp_path, "js", body="console.log(1)\n")
    assert _argv(launch.RunnerLaunch().build(entry, [], None, None)) == [
        "/n",
        str(entry.script_path),
    ]


def test_runner_none_installed_names_candidates_and_config_key(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    entry = _entry(tmp_path, "js", body="console.log(1)\n")
    with pytest.raises(NotExecutableError) as exc:
        launch.RunnerLaunch().build(entry, [], None, None)
    msg = str(exc.value)
    for candidate in ("deno", "bun", "node"):
        assert candidate in msg
    assert "skit config js.runner" in msg  # teaches a real command


def test_runner_describe_uses_preferred_name_without_path_lookup(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    def boom(_name: str) -> str:
        raise AssertionError("describe must not touch PATH")

    monkeypatch.setattr("skit.langs.launch._which", boom)
    entry = _entry(tmp_path, "js", body="console.log(1)\n")
    described = launch.RunnerLaunch().describe(entry, [], None, None)
    assert described.startswith("deno run ")  # ORDER[0], no lookup


def test_runner_preflight_checks_script_and_runner(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("skit.langs.launch._which", _which_map({}))
    entry = _entry(tmp_path, "js", body="console.log(1)\n")
    with pytest.raises(NotExecutableError):
        launch.RunnerLaunch().preflight(entry)


def test_runner_target_is_script_path(tmp_path: Path):
    entry = _entry(tmp_path, "js", body="console.log(1)\n")
    assert launch.RunnerLaunch().target(entry) == entry.script_path


# ==========================================================================
# shebang_program + infer_kind
# ==========================================================================


def _write(tmp_path: Path, name: str, data: bytes, *, executable: bool = False) -> Path:
    p = tmp_path / name
    p.write_bytes(data)
    if executable:
        p.chmod(0o755)
    return p


def test_shebang_plain(tmp_path: Path):
    p = _write(tmp_path, "s", b"#!/bin/bash\necho hi\n")
    assert reg.shebang_program(p) == "bash"


def test_shebang_env_form(tmp_path: Path):
    p = _write(tmp_path, "s", b"#!/usr/bin/env python3\n")
    assert reg.shebang_program(p) == "python3"


def test_shebang_env_dash_s_with_flags(tmp_path: Path):
    p = _write(tmp_path, "s", b"#!/usr/bin/env -S deno run --allow-net\n")
    assert reg.shebang_program(p) == "deno"  # env flags skipped, the program still wins


def test_shebang_none_when_no_shebang(tmp_path: Path):
    p = _write(tmp_path, "s", b"echo hi\n")
    assert reg.shebang_program(p) is None


def test_shebang_none_when_unreadable(tmp_path: Path):
    assert reg.shebang_program(tmp_path / "does-not-exist") is None  # OSError -> None
    assert reg.shebang_program(tmp_path) is None  # a directory is unreadable as a file too


def test_shebang_none_when_empty_hashbang_line(tmp_path: Path):
    p = _write(tmp_path, "s", b"#!\n")
    assert reg.shebang_program(p) is None  # no tokens after #!


def test_shebang_env_with_only_flags_is_none(tmp_path: Path):
    p = _write(tmp_path, "s", b"#!/usr/bin/env -S\n")
    assert reg.shebang_program(p) is None  # env, then nothing but a flag


def test_kind_for_shebang_maps_the_program_or_none(tmp_path: Path):
    """The one shebang→kind mapping, shared by the TUI draft lane and the CLI --edit lane
    (registry.kind_for_shebang). A registered #! program names its kind; an unmapped or
    absent shebang is None — the caller (never this helper) decides the python fallback."""
    bash = _write(tmp_path, "a", b"#!/usr/bin/env bash\necho hi\n")
    assert reg.kind_for_shebang(bash) == "shell"

    node = _write(tmp_path, "b", b"#!/usr/bin/env node\n")
    assert reg.kind_for_shebang(node) == "js"

    py = _write(tmp_path, "c", b"#!/usr/bin/env python3\n")
    assert reg.kind_for_shebang(py) == "python"

    unmapped = _write(tmp_path, "d", b"#!/usr/bin/env cobol\n")
    assert reg.kind_for_shebang(unmapped) is None  # recognized shape, unmapped program

    no_shebang = _write(tmp_path, "e", b"echo hi\n")
    assert reg.kind_for_shebang(no_shebang) is None  # no #! at all


def test_infer_extension_beats_shebang(tmp_path: Path):
    # A .py file whose shebang says bash is still python — the extension is authoritative.
    p = _write(tmp_path, "j.py", b"#!/bin/bash\n", executable=True)
    assert reg.infer_kind(p) == "python"


def test_infer_shebang_beats_exec_bit(tmp_path: Path):
    p = _write(tmp_path, "deploy", b"#!/usr/bin/env bash\necho hi\n", executable=True)
    assert reg.infer_kind(p) == "shell"


def test_infer_unknown_shebang_program_falls_to_exec_bit(tmp_path: Path):
    # A recognized #! shape but an unmapped program: not an interpreted kind, so the fall-through
    # is the executability check (covers the "shebang present, no kind" branch). On POSIX the +x
    # bit makes it an exe; Windows has no execute bit and this extension-less file isn't in PATHEXT,
    # so it honestly stays unknown — the same platform split as test_infer_exec_bit_only_is_exe.
    p = _write(tmp_path, "prog", b"#!/usr/bin/env frobnicator\n", executable=True)
    assert reg.infer_kind(p) == ("unknown" if sys.platform == "win32" else "exe")


def test_infer_exec_bit_only_is_exe(tmp_path: Path):
    if sys.platform == "win32":
        pytest.skip("POSIX execute bit")
    p = _write(tmp_path, "prog", b"opaque bytes\n", executable=True)
    assert reg.infer_kind(p) == "exe"


def test_infer_plain_file_is_unknown(tmp_path: Path):
    p = _write(tmp_path, "notes", b"just text\n")  # no ext, no shebang, no +x
    assert reg.infer_kind(p) == "unknown"


def test_infer_zsh_extension_is_shell(tmp_path: Path):
    p = _write(tmp_path, "x.zsh", b"echo hi\n")
    assert reg.infer_kind(p) == "shell"


def test_infer_r_extension_is_case_insensitive(tmp_path: Path):
    p = _write(tmp_path, "x.R", b'cat("hi")\n')
    assert reg.infer_kind(p) == "r"  # .R lowercases to .r


# ==========================================================================
# needs — preflight / run / missing_needs
# ==========================================================================


def _needs_which(present: set[str]):
    return lambda name: ("/usr/bin/" + name) if name in present else None


def test_preflight_needs_lists_only_missing(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("shutil.which", _needs_which({"bash", "jq"}))  # ffmpeg absent
    entry = _entry(tmp_path, "shell")
    entry.meta.needs = ["jq", "ffmpeg"]
    with pytest.raises(NotExecutableError) as exc:
        launcher.preflight(entry, invoke_cwd=tmp_path)
    msg = str(exc.value)
    assert "ffmpeg" in msg
    assert "jq" not in msg  # a satisfied requirement is never named


def test_run_entry_needs_raises_before_spawn(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("shutil.which", _needs_which({"jq"}))  # ffmpeg absent
    entry = _entry(tmp_path, "shell")
    entry.meta.needs = ["ffmpeg"]
    with pytest.raises(NotExecutableError) as exc:
        launcher.run_entry(entry, invoke_cwd=tmp_path)
    assert "ffmpeg" in str(exc.value)


def test_missing_needs_returns_the_gap(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("shutil.which", _needs_which({"jq"}))
    entry = _entry(tmp_path, "shell")
    entry.meta.needs = ["jq", "ffmpeg"]
    assert launcher.missing_needs(entry) == ["ffmpeg"]


def test_missing_needs_empty_when_all_present(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr("shutil.which", _needs_which({"jq", "ffmpeg"}))
    entry = _entry(tmp_path, "shell")
    entry.meta.needs = ["jq", "ffmpeg"]
    assert launcher.missing_needs(entry) == []


# ==========================================================================
# models — interpreter / needs / parameters round-trip
# ==========================================================================


def test_meta_round_trip_carries_interpreter_needs_parameters():
    meta = ScriptMeta(
        name="e",
        kind="shell",
        interpreter="zsh",
        needs=["jq", "ffmpeg"],
        parameters=[{"name": "WIDTH", "delivery": "env"}],
    )
    restored = ScriptMeta.from_toml_dict(meta.to_toml_dict())
    assert restored.interpreter == "zsh"
    assert restored.needs == ["jq", "ffmpeg"]
    assert restored.parameters == [{"name": "WIDTH", "delivery": "env"}]


def test_meta_omits_empty_needs():
    # The line-94 region only serializes needs when set — an empty list stays absent.
    assert "needs" not in ScriptMeta(name="e", kind="shell").to_toml_dict()


# ==========================================================================
# store.update_needs
# ==========================================================================


def test_update_needs_sets_and_clears(tmp_path: Path):
    sh = tmp_path / "d.sh"
    sh.write_text("#!/bin/bash\necho hi\n", encoding="utf-8")
    store.add_script(sh, kind="shell", name="d")
    entry = store.update_needs("d", ["jq", "ffmpeg"])
    assert entry.meta.needs == ["jq", "ffmpeg"]
    assert store.resolve("d").meta.needs == ["jq", "ffmpeg"]
    cleared = store.update_needs("d", [])
    assert cleared.meta.needs is None  # empty clears to None (minimal meta)


# ==========================================================================
# CLI: add --kind
# ==========================================================================


def test_cli_add_shell_script_records_interpreter(tmp_path: Path):
    sh = tmp_path / "deploy.sh"
    sh.write_text("#!/usr/bin/env zsh\n# Ship it\necho hi\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(sh), "-n", "deploy"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("deploy")
    assert entry.meta.kind == "shell"
    assert entry.meta.interpreter == "zsh"  # shebang outranks the shell default
    assert entry.meta.description == "Ship it"


def test_cli_add_kind_forces_extensionless_file(tmp_path: Path):
    build = tmp_path / "build"  # no extension, no shebang
    build.write_text("echo building\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(build), "--kind", "shell", "-n", "build"])
    assert result.exit_code == 0, result.output
    entry = store.resolve("build")
    assert entry.meta.kind == "shell"
    assert (entry.dir / "script.sh").exists()  # stored under the kind's copy name


def test_cli_add_kind_exe(tmp_path: Path):
    prog = tmp_path / "thing"
    prog.write_text("bytes\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(prog), "--kind", "exe", "-n", "thing"])
    assert result.exit_code == 0, result.output
    assert store.resolve("thing").meta.kind == "exe"


def test_cli_add_kind_unknown_is_usage_error(tmp_path: Path):
    f = tmp_path / "x"
    f.write_text("x\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(f), "--kind", "cobol", "-n", "x"])
    assert result.exit_code == 2
    assert "shell" in result.output  # the usage error lists the valid kinds
    assert store.list_entries() == []


def test_cli_add_kind_and_exe_conflict(tmp_path: Path):
    f = tmp_path / "x"
    f.write_text("x\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(f), "--kind", "shell", "--exe", "-n", "x"])
    assert result.exit_code == 2
    assert store.list_entries() == []


def test_cli_add_command_kind_rejected(tmp_path: Path):
    # command templates are their own path (--cmd); --kind command is not offered.
    f = tmp_path / "x"
    f.write_text("x\n", encoding="utf-8")
    result = runner.invoke(cli.app, ["add", str(f), "--kind", "command", "-n", "x"])
    assert result.exit_code == 2


# ==========================================================================
# CLI: deps --need / --clear-needs / read view
# ==========================================================================


def _shell(tmp_path: Path, name: str = "d") -> None:
    sh = tmp_path / f"{name}.sh"
    sh.write_text("#!/bin/bash\necho hi\n", encoding="utf-8")
    store.add_script(sh, kind="shell", name=name)


def test_deps_need_sets_the_list(tmp_path: Path):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["deps", "d", "--need", "jq", "--need", "ffmpeg"])
    assert result.exit_code == 0, result.output
    assert store.resolve("d").meta.needs == ["jq", "ffmpeg"]


def test_deps_need_replaces_whole_list(tmp_path: Path):
    _shell(tmp_path)
    runner.invoke(cli.app, ["deps", "d", "--need", "jq"])
    runner.invoke(cli.app, ["deps", "d", "--need", "ffmpeg"])
    assert store.resolve("d").meta.needs == ["ffmpeg"]  # replaced, not appended


def test_deps_clear_needs(tmp_path: Path):
    _shell(tmp_path)
    runner.invoke(cli.app, ["deps", "d", "--need", "jq"])
    result = runner.invoke(cli.app, ["deps", "d", "--clear-needs"])
    assert result.exit_code == 0
    assert store.resolve("d").meta.needs is None


def test_deps_need_and_clear_needs_conflict(tmp_path: Path):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["deps", "d", "--need", "jq", "--clear-needs"])
    assert result.exit_code == 2
    assert "not both" in result.output


def test_deps_need_works_on_python_too(tmp_path: Path):
    py = tmp_path / "a.py"
    py.write_text("print(1)\n", encoding="utf-8")
    store.add_python(py, name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--need", "ffmpeg"])
    assert result.exit_code == 0
    assert store.resolve("a").meta.needs == ["ffmpeg"]


def test_deps_dep_on_shell_is_refused(tmp_path: Path):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["deps", "d", "--dep", "requests"])
    # A refused flag is a usage error (2), the same code `skit add` gives.
    assert result.exit_code == 2
    assert "doesn't take package dependencies" in result.output


def test_deps_read_view_shows_needs_for_shell(tmp_path: Path):
    _shell(tmp_path)
    store.update_needs("d", ["jq"])
    result = runner.invoke(cli.app, ["deps", "d"])
    assert result.exit_code == 0
    assert "jq" in result.output


def test_deps_json_view_includes_needs(tmp_path: Path):
    _shell(tmp_path)
    store.update_needs("d", ["jq"])
    result = runner.invoke(cli.app, ["deps", "d", "--json"])
    assert result.exit_code == 0
    import json

    doc = json.loads(result.output)
    assert doc["needs"] == ["jq"]


def test_deps_read_view_needs_dash_when_empty(tmp_path: Path):
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["deps", "d"])
    assert result.exit_code == 0
    assert "—" in result.output  # the empty-needs dash


# ==========================================================================
# CLI: doctor / show needs surfaces
# ==========================================================================


def test_doctor_flags_missing_needs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    _shell(tmp_path)
    store.update_needs("d", ["ffmpeg"])
    monkeypatch.setattr("shutil.which", _needs_which(set()))  # nothing present
    result = runner.invoke(cli.app, ["doctor"])
    assert "ffmpeg" in result.output
    assert "d" in result.output


def test_doctor_json_needs_missing(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    _shell(tmp_path)
    store.update_needs("d", ["ffmpeg"])
    monkeypatch.setattr("shutil.which", _needs_which(set()))
    result = runner.invoke(cli.app, ["doctor", "--json"])
    import json

    doc = json.loads(result.output)
    assert doc["needs_missing"] == {"d": ["ffmpeg"]}


def test_show_human_prints_needs_line(tmp_path: Path):
    _shell(tmp_path)
    store.update_needs("d", ["jq", "ffmpeg"])
    result = runner.invoke(cli.app, ["show", "d"])
    assert result.exit_code == 0
    assert "Needs:" in result.output
    assert "jq" in result.output


def test_show_json_includes_needs(tmp_path: Path):
    _shell(tmp_path)
    store.update_needs("d", ["jq"])
    result = runner.invoke(cli.app, ["show", "d", "--json"])
    import json

    doc = json.loads(result.output)
    assert doc["needs"] == ["jq"]


def test_show_interpreted_header_and_source(tmp_path: Path):
    # A shell entry: (kind · mode) header, a Source line (it has an original file), and
    # the run hint — all render sensibly through the capability checks.
    _shell(tmp_path)
    result = runner.invoke(cli.app, ["show", "d"])
    assert result.exit_code == 0
    assert "Shell · copy" in result.output  # human header uses the translated kind label
    assert "Source:" in result.output
    assert "skit run d" in result.output


# ==========================================================================
# CLI: edit refusal is kind-neutral
# ==========================================================================


def test_edit_program_refusal_is_kind_neutral(tmp_path: Path):
    prog = tmp_path / "thing"
    prog.write_text("bytes\n", encoding="utf-8")
    store.add_exe(prog, name="thing")
    result = runner.invoke(cli.app, ["edit", "thing"])
    assert result.exit_code == 1
    assert "no editable source" in result.output
    assert "Python" not in result.output  # the old python-only wording is gone


def test_edit_command_refusal_is_kind_neutral(tmp_path: Path):
    store.add_command("echo hi", name="c")
    result = runner.invoke(cli.app, ["edit", "c"])
    assert result.exit_code == 1
    assert "no editable source" in result.output


# ==========================================================================
# E2E (POSIX): the overlay reaches a real child
# ==========================================================================


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell E2E")
def test_e2e_run_shell_script(tmp_path: Path, capfd: pytest.CaptureFixture[str]):
    sh = tmp_path / "hi.sh"
    sh.write_text('#!/bin/bash\necho "shell-ran-ok"\n', encoding="utf-8")
    store.add_script(sh, kind="shell", name="hi")
    result = runner.invoke(cli.app, ["run", "hi"])
    assert result.exit_code == 0, result.output
    assert "shell-ran-ok" in capfd.readouterr().out


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell E2E")
def test_e2e_run_shell_env_param_reaches_child(tmp_path: Path, capfd: pytest.CaptureFixture[str]):
    sh = tmp_path / "w.sh"
    sh.write_text('#!/bin/bash\necho "w=$WIDTH"\n', encoding="utf-8")
    store.add_script(sh, kind="shell", name="w")
    store.write_parameters("w", [ParamDecl(name="WIDTH", binding="none", delivery="env")])
    result = runner.invoke(cli.app, ["run", "w", "--set", "WIDTH=800"])
    assert result.exit_code == 0, result.output
    assert "w=800" in capfd.readouterr().out  # the env overlay reached the real child


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell E2E")
def test_e2e_dry_run_shows_interpreter_and_script(tmp_path: Path):
    sh = tmp_path / "d.sh"
    sh.write_text("#!/bin/bash\necho hi\n", encoding="utf-8")
    store.add_script(sh, kind="shell", name="d")
    result = runner.invoke(cli.app, ["run", "d", "--dry-run"])
    assert result.exit_code == 0, result.output
    # rich soft-wraps the long tmp path across lines, so join before matching.
    flat = result.output.replace("\n", "")
    assert "bash" in flat
    assert "script.sh" in flat


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell E2E")
def test_e2e_run_reference_mode_shell(tmp_path: Path, capfd: pytest.CaptureFixture[str]):
    sh = tmp_path / "ref.sh"
    sh.write_text('#!/bin/bash\necho "ref-ran"\n', encoding="utf-8")
    store.add_script(sh, kind="shell", name="ref", mode="reference")
    result = runner.invoke(cli.app, ["run", "ref"])
    assert result.exit_code == 0, result.output
    assert "ref-ran" in capfd.readouterr().out
