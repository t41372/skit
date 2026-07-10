"""Mutation-strength behavioural tests for shim.py, launcher.py, uvman.py.

These pin down behaviour the base suites exercise but never assert: exact user-facing
error messages (a message-string mutant like ``ShimError(str(exc)) -> ShimError(None)``
survives when nothing reads the message), token tables, replacement ordering, subprocess
contracts (cwd/env/exit-code pass-through), and the download pipeline's URL/timeout/
progress-output contract. Zero network: every download path is stubbed.
"""

from __future__ import annotations

import ast
import hashlib
import io
import subprocess
import sys
import tarfile
import urllib.error
import zipfile
from pathlib import Path

import pytest

from skit import shim, uvman
from skit.metawriter import ParamSpec


@pytest.fixture(autouse=True)
def _force_english_locale(monkeypatch: pytest.MonkeyPatch) -> None:
    """Pin gettext output to English regardless of the host locale (LC_ALL/LANG).

    Several tests in this file assert on exact English message text; without this,
    they fail under a non-English locale even though the conftest i18n-reset fixture
    doesn't touch LC_*/LANG itself.
    """
    monkeypatch.setenv("SKIT_LANG", "en")


def spec(
    name: str, *, kind: str = "const", type: str = "str", order: int = -1, secret: bool = False
) -> ParamSpec:
    return ParamSpec(name=name, kind=kind, type=type, order=order, secret=secret)


def _run_injected(source: str, stdin: str = "") -> str:
    """Run the injected output in a subprocess and return stdout (behaviour verification)."""
    proc = subprocess.run(
        [sys.executable, "-c", source],
        input=stdin,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    return proc.stdout


@pytest.fixture
def py_entry(tmp_path: Path):
    from skit import store

    p = tmp_path / "s.py"
    p.write_text("print('ok')\n", encoding="utf-8")
    return store.add_python(p)


# =========================================================================================
# shim: _coerce_bool / _coerce_float / _coerce
# =========================================================================================


@pytest.mark.parametrize(
    ("raw", "expected"),
    [
        ("true", True),
        ("1", True),
        ("yes", True),
        ("y", True),
        ("on", True),
        ("  YES  ", True),  # strip + lower normalisation
        ("false", False),
        ("0", False),
        ("no", False),
        ("n", False),
        ("off", False),
        ("  Off  ", False),
    ],
)
def test_coerce_bool_accepts_every_documented_token(raw: str, expected: bool) -> None:
    """Each documented boolean token must map to its exact value through injection."""
    out = shim.inject("FLAG = True\n", [spec("FLAG", type="bool")], {"FLAG": raw})
    assert f"FLAG = {expected!r}" in out


def test_coerce_bool_error_carries_the_offending_value() -> None:
    with pytest.raises(ValueError, match="maybe"):
        shim._coerce_bool("maybe")


@pytest.mark.parametrize("bad", ["inf", "nan"])
def test_coerce_float_rejects_non_literal_floats_naming_the_value(bad: str) -> None:
    """repr(inf/nan) is not a valid Python literal; the error must name the rejected value."""
    with pytest.raises(ValueError, match=bad):
        shim._coerce_float(bad)


def test_coerce_failure_message_names_value_and_type() -> None:
    with pytest.raises(shim.ShimError) as exc_info:
        shim.inject("RETRIES = 3\n", [spec("RETRIES", type="int")], {"RETRIES": "abc"})
    assert str(exc_info.value) == "'abc' -> int"


# =========================================================================================
# shim: inject error reporting and spec-loop continuation
# =========================================================================================


def test_inject_syntax_error_message_matches_the_parse_error() -> None:
    """The ShimError must carry the SyntaxError's own text so the user sees where it broke."""
    src = "def broken(:\n"
    try:
        ast.parse(src)
        raise AssertionError("source must not parse")
    except SyntaxError as exc:
        expected = str(exc)
    with pytest.raises(shim.ShimError) as exc_info:
        shim.inject(src, [spec("X")], {"X": "1"})
    assert str(exc_info.value) == expected


def test_spec_without_value_does_not_stop_later_specs() -> None:
    """A spec with no form value is skipped, not a loop-terminator."""
    out = shim.inject("CITY = 'a'\n", [spec("NOVAL"), spec("CITY")], {"CITY": "b"})
    assert "CITY = 'b'" in out


def test_input_order_equal_to_call_count_is_drift() -> None:
    """order == len(input_calls) is one past the last call: definition drift, not a queue slot."""
    src = "x = input()\nprint(x)\n"
    with pytest.raises(shim.ShimError) as exc_info:
        shim.inject(src, [spec("input-2", kind="input", order=1)], {"input-2": "v"})
    assert str(exc_info.value) == "input-2"


def test_input_spec_does_not_stop_later_const_specs() -> None:
    out = shim.inject(
        "x = input()\nCITY = 'a'\nprint(x, CITY)\n",
        [spec("input-1", kind="input", order=0), spec("CITY")],
        {"input-1": "v", "CITY": "b"},
    )
    assert "CITY = 'b'" in out
    assert "# skit:shim" in out


def test_every_drifted_name_is_reported_comma_joined() -> None:
    """All drifted parameters must be listed (not just the first), joined with ', '."""
    with pytest.raises(shim.ShimError) as exc_info:
        shim.inject("A = 1\n", [spec("GONE1"), spec("GONE2")], {"GONE1": "x", "GONE2": "y"})
    assert str(exc_info.value) == "GONE1, GONE2"


# =========================================================================================
# shim: preamble placement
# =========================================================================================


def test_preamble_goes_directly_after_docstring_before_first_statement() -> None:
    """With a docstring and the first input() immediately after it, the preamble must land
    between them — one line too low and the input() runs before the interception queue."""
    src = '"""d"""\nx = input("p: ")\nprint(x)\n'
    out = shim.inject(src, [spec("input-1", kind="input", order=0)], {"input-1": "queued"})
    lines = out.splitlines()
    assert lines[0] == '"""d"""'
    assert lines[1].endswith("# skit:shim")
    assert "queued" in _run_injected(out)


def test_preamble_inserted_above_decorators() -> None:
    """Decorators sit above the def's lineno; inserting between them is a SyntaxError."""
    src = "@(lambda g: g)\ndef f():\n    return input('p: ')\nprint(f())\n"
    out = shim.inject(src, [spec("input-1", kind="input", order=0)], {"input-1": "deco"})
    lines = out.splitlines()
    assert lines[0].endswith("# skit:shim")
    assert lines[1] == "@(lambda g: g)"
    assert "deco" in _run_injected(out)


def test_preamble_line_index_contract_violation_raises_named_assertion() -> None:
    """A module with nothing after the docstring violates the caller contract; the guard's
    message must state the invariant."""
    tree = ast.parse('"""only a docstring"""')
    with pytest.raises(AssertionError) as exc_info:
        shim._preamble_line_index(tree)
    assert str(exc_info.value) == "unreachable: caller guarantees a stmt follows the preamble"


# =========================================================================================
# shim: _node_replacement guard
# =========================================================================================


def test_node_replacement_rejects_node_missing_either_end_position() -> None:
    """A node missing only one of end_lineno/end_col_offset is still unusable (or-guard),
    and the error names the problem."""
    stmt = ast.parse("X = 1").body[0]
    assert isinstance(stmt, ast.Assign)
    node = stmt.value
    node.end_lineno = None
    with pytest.raises(shim.ShimError) as exc_info:
        shim._node_replacement(node, "2")
    assert str(exc_info.value) == "missing node span"


# =========================================================================================
# shim: _apply replacement ordering and multi-line spans
# =========================================================================================


def test_two_replacements_on_one_line_applied_right_to_left() -> None:
    """Applying left-to-right would shift the second span after the first grows the line."""
    src = "A = 1; B = 2\nprint(A, B)\n"
    out = shim.inject(src, [spec("A", type="int"), spec("B", type="int")], {"A": "100", "B": "200"})
    assert out.splitlines()[0] == "A = 100; B = 200"
    assert "100 200" in _run_injected(out)


def test_multiline_span_replacement_exact_output() -> None:
    """A value spanning two lines collapses to one merged line; every other byte survives."""
    src = 'X = (\n    "old"\n    "also old"\n)\nprint(X)\n'
    out = shim.inject(src, [spec("X")], {"X": "new"})
    assert out == "X = (\n    'new'\n)\nprint(X)\n"


# =========================================================================================
# shim: write_injected placement and encoding
# =========================================================================================


def test_write_injected_lands_outside_entry_dir(tmp_path: Path) -> None:
    """The injected temp file may contain plaintext secrets, so it must NOT be created inside the
    persistent store entry_dir (a crash there would leave a secret file next to script.py forever);
    it goes to the OS temp dir instead. Name prefix/suffix, exact content and 0600 perms still hold."""
    p = shim.write_injected(tmp_path, "print('x')\n")
    try:
        assert p.parent != tmp_path
        assert tmp_path not in p.parents  # not anywhere under the persistent store dir
        assert p.name.startswith(".injected-")
        assert p.name.endswith(".py")
        assert p.read_text(encoding="utf-8") == "print('x')\n"
        if sys.platform != "win32":
            assert (p.stat().st_mode & 0o777) == 0o600
    finally:
        p.unlink()


def test_write_injected_requests_utf8_explicitly(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The injected file must be UTF-8 regardless of the user's locale (a C/POSIX locale
    would corrupt non-ASCII const values). The process locale can't be swapped mid-run, so
    assert the explicit encoding request on the write handle plus a CJK round-trip."""
    import os
    from typing import Any

    seen: dict[str, Any] = {}
    real_fdopen = os.fdopen

    def spy(fd: int, *args: Any, **kwargs: Any) -> Any:
        seen.update(kwargs)
        return real_fdopen(fd, *args, **kwargs)

    monkeypatch.setattr(os, "fdopen", spy)
    p = shim.write_injected(tmp_path, "CITY = '臺北'\n")
    try:
        assert str(seen.get("encoding") or "").lower() == "utf-8"
        assert p.read_text(encoding="utf-8") == "CITY = '臺北'\n"
    finally:
        p.unlink()


# =========================================================================================
# launcher: uv discovery and message contracts
# =========================================================================================


def test_find_uv_queries_path_for_the_exact_name_uv(monkeypatch: pytest.MonkeyPatch) -> None:
    from skit import launcher

    monkeypatch.setattr("shutil.which", lambda name: "/path/to/uv" if name == "uv" else None)
    assert launcher.find_uv() == "/path/to/uv"


def test_ensure_uv_returns_found_uv_without_downloading(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """When find_uv locates a binary, ensure_uv must return it and never touch the
    download path (a network fallback here would be a first-run regression)."""
    from skit import launcher

    monkeypatch.setattr(launcher, "find_uv", lambda: "/found/uv")

    def never(**_kw: object) -> str:
        raise AssertionError("must not download when uv was found")

    monkeypatch.setattr(uvman, "ensure_uv_downloaded", never)
    assert launcher.ensure_uv() == "/found/uv"


def test_ensure_uv_failure_message_includes_guidance_and_cause(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from skit import launcher

    monkeypatch.setattr(launcher, "find_uv", lambda: None)

    def boom(**_kw: object) -> str:
        raise uvman.UvDownloadError("boom")

    monkeypatch.setattr(uvman, "ensure_uv_downloaded", boom)
    with pytest.raises(launcher.LaunchError) as exc_info:
        launcher.ensure_uv()
    assert str(exc_info.value) == (
        "uv not found. Install it (https://docs.astral.sh/uv/) or run skit doctor for"
        " guidance. (boom)"
    )


def test_build_python_missing_script_message_names_the_path(
    py_entry, monkeypatch: pytest.MonkeyPatch
) -> None:
    from skit import launcher

    monkeypatch.setattr(launcher, "find_uv", lambda: "/fake/uv")
    py_entry.script_path.unlink()
    with pytest.raises(launcher.LaunchError) as exc_info:
        launcher.build_command(py_entry)
    assert str(exc_info.value) == f"The script file doesn't exist: {py_entry.script_path}"


def test_build_exe_missing_message_names_the_path(tmp_path: Path) -> None:
    from skit import launcher, store

    exe = tmp_path / "tool"
    exe.touch()
    entry = store.add_exe(exe)
    exe.unlink()
    with pytest.raises(launcher.LaunchError) as exc_info:
        launcher.build_command(entry)
    assert str(exc_info.value) == f"The executable doesn't exist: {entry.meta.source}"


def test_build_command_unknown_kind_message_names_the_kind(py_entry) -> None:
    from skit import launcher

    py_entry.meta.kind = "weird"
    with pytest.raises(launcher.LaunchError) as exc_info:
        launcher.build_command(py_entry)
    assert str(exc_info.value) == "Unknown entry kind: weird"


# =========================================================================================
# launcher: _build_shell template semantics
# =========================================================================================


def test_build_shell_missing_params_message_lists_all_names() -> None:
    from skit import launcher, store

    entry = store.add_command("echo {a} {b}", name="two-params")
    with pytest.raises(launcher.LaunchError) as exc_info:
        launcher.build_command(entry, values={})
    assert str(exc_info.value) == "Missing parameter values: a, b"


def test_build_shell_substitutes_uppercase_placeholders() -> None:
    """Placeholder names may be uppercase; the launcher regex must match what
    store.extract_placeholders matched at add time."""
    from skit import launcher, store

    entry = store.add_command("echo {NAME}", name="upper")
    assert launcher.build_command(entry, values={"NAME": "x"}) == "echo x"


def test_build_shell_leaves_unknown_placeholder_verbatim() -> None:
    """A placeholder in the template that isn't a declared param (drift) must be left
    exactly as written — never dropped, never de-braced."""
    from skit import launcher, store

    entry = store.add_command("echo {known} {unknown}", name="drift")
    entry.meta.params = ["known"]
    assert launcher.build_command(entry, values={"known": "v"}) == "echo v {unknown}"


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX quoting branch")
def test_build_shell_appends_extra_args_with_single_space_and_shell_quoting() -> None:
    from skit import launcher, store

    entry = store.add_command("echo hello", name="spacing")
    assert launcher.build_command(entry, ["world", "a b"]) == "echo hello world 'a b'"


# =========================================================================================
# launcher: run_entry argument forwarding and subprocess contract
# =========================================================================================


class _FakeProc:
    returncode = 0


def _stub_run(monkeypatch: pytest.MonkeyPatch) -> dict[str, object]:
    from skit import launcher

    captured: dict[str, object] = {}

    def fake_run(cmd: object, **kwargs: object) -> _FakeProc:
        captured["cmd"] = cmd
        captured.update(kwargs)
        return _FakeProc()

    monkeypatch.setattr(launcher.subprocess, "run", fake_run)
    return captured


def test_run_entry_forwards_extra_args_and_values(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from skit import launcher, store

    entry = store.add_command("echo {name}", name="forward")
    captured = _stub_run(monkeypatch)
    code = launcher.run_entry(entry, ["extra1"], values={"name": "val"}, invoke_cwd=tmp_path)
    assert code == 0
    assert captured["cmd"] == "echo val extra1"


def test_run_entry_forwards_script_override(
    py_entry, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from skit import launcher

    monkeypatch.setattr(launcher, "find_uv", lambda: "/fake/uv")
    override = tmp_path / "inj.py"
    override.write_text("print(1)\n", encoding="utf-8")
    captured = _stub_run(monkeypatch)
    launcher.run_entry(py_entry, script_override=override, invoke_cwd=tmp_path)
    cmd = captured["cmd"]
    assert isinstance(cmd, list)
    assert "--no-project" in cmd
    assert str(override) in cmd


def test_run_entry_uses_invoke_cwd_for_invoke_policy(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The child must run in the directory the user invoked from — not the test process cwd."""
    from skit import launcher, store

    entry = store.add_command("echo hi", name="cwd-check")
    entry.meta.workdir = "invoke"
    wd = tmp_path / "wd"
    wd.mkdir()
    captured = _stub_run(monkeypatch)
    launcher.run_entry(entry, invoke_cwd=wd)
    assert captured["cwd"] == wd


def test_run_entry_missing_workdir_message_names_the_path(tmp_path: Path) -> None:
    from skit import launcher, store

    entry = store.add_command("echo hi", name="badwd")
    entry.meta.workdir = "/nonexistent/skit-test-path"
    with pytest.raises(launcher.LaunchError) as exc_info:
        launcher.run_entry(entry, invoke_cwd=tmp_path)
    assert str(exc_info.value) == (
        "The working directory doesn't exist: /nonexistent/skit-test-path"
    )


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell")
def test_run_entry_shell_child_gets_mirror_env_and_exit_code_passthrough(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """End-to-end shell branch: the child sees skit's mirror env overlay, and a nonzero
    exit code is returned (never raised)."""
    from skit import config, launcher, store

    monkeypatch.delenv("UV_DEFAULT_INDEX", raising=False)
    monkeypatch.delenv("UV_PYTHON_INSTALL_MIRROR", raising=False)
    config.save_mirror(config.preset("tsinghua"))
    outfile = tmp_path / "env.txt"
    entry = store.add_command(
        f'printf "%s" "$UV_DEFAULT_INDEX" > "{outfile}"; exit 7', name="env-exit"
    )
    entry.meta.params = []  # the template braces above are quotes only, but be explicit
    entry.meta.workdir = "invoke"
    code = launcher.run_entry(entry, invoke_cwd=tmp_path)
    assert code == 7
    assert outfile.read_text(encoding="utf-8") == config.PYPI_PRESETS["tsinghua"]


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell script")
def test_run_entry_argv_nonzero_exit_code_returned_not_raised(tmp_path: Path) -> None:
    from skit import launcher, store

    exe = tmp_path / "fail.sh"
    exe.write_text("#!/bin/sh\nexit 5\n", encoding="utf-8")
    exe.chmod(0o755)
    entry = store.add_exe(exe)
    assert launcher.run_entry(entry, invoke_cwd=tmp_path) == 5


# =========================================================================================
# uvman: _ask_consent
# =========================================================================================


def _tty(monkeypatch: pytest.MonkeyPatch, *, stdin: bool, stderr: bool) -> None:
    monkeypatch.setattr("sys.stdin", io.StringIO(""), raising=False)
    monkeypatch.setattr("sys.stdin.isatty", lambda: stdin, raising=False)
    monkeypatch.setattr("sys.stderr.isatty", lambda: stderr, raising=False)


def test_consent_requires_both_streams_tty_no_prompt_otherwise(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    """stdin not a tty (pipe) with stderr a tty must auto-consent without prompting or
    blocking on input()."""
    _tty(monkeypatch, stdin=False, stderr=True)

    def never() -> str:
        raise AssertionError("input() must not be called")

    monkeypatch.setattr("builtins.input", never)
    assert uvman._ask_consent(Path("/tmp/x")) is True
    assert capsys.readouterr().err == ""


def test_consent_prompt_exact_text_on_stderr(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    _tty(monkeypatch, stdin=True, stderr=True)
    monkeypatch.setattr("builtins.input", lambda: "n")
    assert uvman._ask_consent(Path("/pb")) is False
    cap = capsys.readouterr()
    assert cap.out == ""
    assert cap.err == (
        "skit needs Astral's uv to run Python scripts, but it wasn't found on this system."
        f" Download uv {uvman.UV_VERSION} into skit's private directory (/pb)?"
        " This won't touch your PATH or global environment. [Y/n] "
    )


def test_consent_prompt_flushed_before_blocking_on_input(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The prompt must be flushed to stderr before input() blocks, or an interactive user
    stares at a silent terminal."""
    events: list[str] = []

    class FakeErr:
        def isatty(self) -> bool:
            return True

        def write(self, s: str) -> int:
            events.append(f"write:{s}")
            return len(s)

        def flush(self) -> None:
            events.append("flush")

    def answer() -> str:
        events.append("input")
        return "n"

    monkeypatch.setattr("sys.stdin", io.StringIO(""), raising=False)
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("sys.stderr", FakeErr())
    monkeypatch.setattr("builtins.input", answer)
    assert uvman._ask_consent(Path("/pb")) is False
    assert "flush" in events
    assert events.index("flush") < events.index("input")


# =========================================================================================
# uvman: _triple / _verify_checksum message contracts
# =========================================================================================


def test_triple_unsupported_message_exact(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("platform.machine", lambda: "mips")
    monkeypatch.setattr("sys.platform", "linux")
    with pytest.raises(uvman.UvDownloadError) as exc_info:
        uvman._triple()
    assert str(exc_info.value) == "Unsupported platform: linux/mips"


def test_verify_checksum_unpinned_triple_message_exact(tmp_path: Path) -> None:
    archive = tmp_path / "uv.tar.gz"
    archive.write_bytes(b"x")
    with pytest.raises(uvman.UvDownloadError) as exc_info:
        uvman._verify_checksum(archive, "sparc-unknown-linux-gnu")
    assert str(exc_info.value) == (
        "No pinned checksum for platform sparc-unknown-linux-gnu; refusing to run an unverified uv."
    )


def test_verify_checksum_mismatch_message_exact(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    data = b"tampered"
    pinned = "00" * 32
    triple = "x86_64-unknown-linux-gnu"
    monkeypatch.setattr(uvman, "_UV_SHA256", {triple: pinned})
    archive = tmp_path / "uv.tar.gz"
    archive.write_bytes(data)
    with pytest.raises(uvman.UvDownloadError) as exc_info:
        uvman._verify_checksum(archive, triple)
    assert str(exc_info.value) == (
        "Downloaded uv failed its checksum (the mirror may be compromised or the file"
        f" corrupt). Expected {pinned}, got {hashlib.sha256(data).hexdigest()}."
    )


# =========================================================================================
# uvman: _extract_uv
# =========================================================================================


def test_extract_uv_windows_extracts_uv_exe(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """On win32 the archive member and destination must both be exactly 'uv.exe'."""
    monkeypatch.setattr("sys.platform", "win32")
    archive = tmp_path / "uv.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        zf.writestr("uv-dir/uv.exe", "win-bin")
    dest = uvman._extract_uv(archive, tmp_path / "dest")
    assert dest == tmp_path / "dest" / "uv.exe"
    assert dest.read_text(encoding="utf-8") == "win-bin"


def test_extract_uv_targz_creates_nested_dest_dir(tmp_path: Path) -> None:
    """tar.gz branch: extraction happens in the temp dir (not the cwd) and dest_dir is
    created with all missing parents."""
    exe_name = "uv.exe" if sys.platform == "win32" else "uv"
    member = tmp_path / exe_name
    member.write_text("tar-bin", encoding="utf-8")
    archive = tmp_path / "uv.tar.gz"
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(member, arcname=f"uv-1.0/{exe_name}")
    dest_dir = tmp_path / "deep" / "bin"  # two missing levels: requires parents=True
    dest = uvman._extract_uv(archive, dest_dir)
    assert dest == dest_dir / exe_name
    assert dest.read_text(encoding="utf-8") == "tar-bin"
    if sys.platform != "win32":
        assert dest.stat().st_mode & 0o755 == 0o755


def test_extract_uv_into_existing_dest_dir(tmp_path: Path) -> None:
    """A pre-existing destination directory must be accepted (exist_ok), not an error."""
    exe_name = "uv.exe" if sys.platform == "win32" else "uv"
    archive = tmp_path / "uv.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        zf.writestr(f"d/{exe_name}", "zip-bin")
    dest_dir = tmp_path / "dest"
    dest_dir.mkdir()
    dest = uvman._extract_uv(archive, dest_dir)
    assert dest.read_text(encoding="utf-8") == "zip-bin"


def test_extract_uv_no_binary_message_exact(tmp_path: Path) -> None:
    member = tmp_path / "README.txt"
    member.write_text("nothing\n", encoding="utf-8")
    archive = tmp_path / "empty.tar.gz"
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(member, arcname="README.txt")
    with pytest.raises(uvman.UvDownloadError) as exc_info:
        uvman._extract_uv(archive, tmp_path / "dest")
    assert str(exc_info.value) == f"No uv binary found inside the archive: {archive}"


# =========================================================================================
# uvman: ensure_uv_downloaded
# =========================================================================================


def test_ensure_uv_downloaded_windows_dest_is_uv_exe(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """On win32 the managed binary is exactly 'uv.exe'; an existing one short-circuits."""
    monkeypatch.setattr("sys.platform", "win32")
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "uv.exe").touch()
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: bin_dir)

    def boom(*_a: object, **_kw: object) -> str:
        raise AssertionError("must not reach consent/download when uv.exe exists")

    monkeypatch.setattr(uvman, "_ask_consent", boom)
    monkeypatch.setattr(uvman, "download_url", boom)
    assert uvman.ensure_uv_downloaded() == str(bin_dir / "uv.exe")


def test_ensure_uv_downloaded_posix_dest_is_uv(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """On POSIX the managed binary is exactly 'uv'; an existing one short-circuits
    without consent or download."""
    if sys.platform == "win32":
        pytest.skip("POSIX exe name branch")
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "uv").touch()
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: bin_dir)

    def boom(*_a: object, **_kw: object) -> str:
        raise AssertionError("must not reach consent/download when uv exists")

    monkeypatch.setattr(uvman, "_ask_consent", boom)
    monkeypatch.setattr(uvman, "download_url", boom)
    assert uvman.ensure_uv_downloaded() == str(bin_dir / "uv")


def test_consent_receives_private_bin_dir_and_declined_message_exact(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bin_dir = tmp_path / "bin"
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: bin_dir)

    def no_download(*_a: object, **_kw: object) -> str:
        raise AssertionError("a declined consent must never reach the download")

    monkeypatch.setattr(uvman, "download_url", no_download)
    seen: dict[str, Path] = {}

    def consent(dest: Path) -> bool:
        seen["dest"] = dest
        return False

    monkeypatch.setattr(uvman, "_ask_consent", consent)
    with pytest.raises(uvman.UvDeclinedError) as exc_info:
        uvman.ensure_uv_downloaded()
    assert seen["dest"] == bin_dir
    assert str(exc_info.value) == (
        "Download declined. Install uv yourself"
        " (https://docs.astral.sh/uv/getting-started/installation/)"
        " and skit will pick it up automatically."
    )


class _StderrRecorder:
    def __init__(self) -> None:
        self.events: list[tuple[str, str]] = []

    def isatty(self) -> bool:
        return False

    def write(self, s: str) -> int:
        self.events.append(("write", s))
        return len(s)

    def flush(self) -> None:
        self.events.append(("flush", ""))

    @property
    def text(self) -> str:
        return "".join(s for kind, s in self.events if kind == "write")

    @property
    def flushes(self) -> int:
        return sum(1 for kind, _ in self.events if kind == "flush")


def test_ensure_uv_downloaded_happy_path_full_contract(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    """One mocked end-to-end download asserting the whole non-quiet contract:
    - download_url is built from the computed platform triple;
    - urlopen gets that exact URL with the mandatory 60s timeout;
    - both progress messages go to stderr, exactly worded, each flushed;
    - stdout stays clean (reserved for the script's output);
    - the returned path is the extracted binary."""
    data = b"known-good-archive"
    triple = "x86_64-unknown-linux-gnu"
    monkeypatch.setattr(uvman, "_triple", lambda: triple)
    monkeypatch.setattr(uvman, "_UV_SHA256", {triple: hashlib.sha256(data).hexdigest()})
    bin_dir = tmp_path / "bin"
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: bin_dir)

    real_download_url = uvman.download_url
    seen: dict[str, object] = {}

    def du_spy(triple_arg: str | None = None) -> str:
        seen["triple"] = triple_arg
        return real_download_url(triple_arg)

    monkeypatch.setattr(uvman, "download_url", du_spy)

    def fake_urlopen(url: str, timeout: int | None = None) -> io.BytesIO:
        seen["url"] = url
        seen["timeout"] = timeout
        return io.BytesIO(data)

    monkeypatch.setattr("urllib.request.urlopen", fake_urlopen)
    monkeypatch.setattr(uvman, "_extract_uv", lambda _archive, dest_dir: dest_dir / "uv")
    rec = _StderrRecorder()
    monkeypatch.setattr("sys.stderr", rec)  # also makes _ask_consent non-interactive

    result = uvman.ensure_uv_downloaded()

    assert result == str(bin_dir / "uv")
    assert seen["triple"] == triple
    assert seen["url"] == real_download_url(triple)
    assert seen["timeout"] == 60
    assert f"First run — downloading uv {uvman.UV_VERSION}…\n" in rec.text
    assert f"uv installed at: {bin_dir / 'uv'}\n" in rec.text
    assert rec.flushes >= 2  # each progress message is flushed as it happens
    assert capsys.readouterr().out == ""


def test_ensure_uv_quiet_prints_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    data = b"quiet-archive"
    triple = "x86_64-unknown-linux-gnu"
    monkeypatch.setattr(uvman, "_triple", lambda: triple)
    monkeypatch.setattr(uvman, "_UV_SHA256", {triple: hashlib.sha256(data).hexdigest()})
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: tmp_path / "bin")
    monkeypatch.setattr("urllib.request.urlopen", lambda _url, timeout=None: io.BytesIO(data))
    monkeypatch.setattr(uvman, "_extract_uv", lambda _archive, dest_dir: dest_dir / "uv")
    uvman.ensure_uv_downloaded(quiet=True)
    cap = capsys.readouterr()
    assert cap.out == ""
    assert cap.err == ""


def test_ensure_uv_generic_failure_message_exact(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("skit.uvman.private_bin_dir", lambda: tmp_path / "bin")

    def fail(*_a: object, **_kw: object) -> io.BytesIO:
        raise urllib.error.URLError("connection refused")

    monkeypatch.setattr("urllib.request.urlopen", fail)
    with pytest.raises(uvman.UvDownloadError) as exc_info:
        uvman.ensure_uv_downloaded(quiet=True)
    assert str(exc_info.value) == "Failed to download uv: <urlopen error connection refused>"
