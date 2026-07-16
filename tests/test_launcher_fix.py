"""Regression tests for four launcher.py bugs found in review (batch3-F):

1. (:126) double-brace unescape ran over the whole command AFTER substitution, corrupting any
   placeholder value that itself contained "{{" or "}}".
2. (:226) a child killed by a signal returned subprocess's raw negative returncode verbatim.
3. (:86) _build_python resolved/downloaded uv before checking the script exists.
4. (:119) command-template placeholder values were substituted unquoted under shell=True.

conftest.py's autouse _isolate_skit_dirs fixture already points the store at an isolated tmp_path,
so no per-file isolation fixture is needed here.
"""

from __future__ import annotations

import shlex
import sys
from pathlib import Path

import pytest


@pytest.fixture
def py_entry(tmp_path: Path):
    from skit import store

    p = tmp_path / "s.py"
    p.write_text("print('ok')\n", encoding="utf-8")
    return store.add_python(p)


# ---------- (1) double-brace unescape must not corrupt substituted values ----------


def test_placeholder_value_with_double_braces_round_trips():
    """A value containing a literal "{{"/"}}" (e.g. a Jinja/Go-template fragment) must survive
    substitution unchanged — the old two-pass implementation (substitute, then str.replace the
    whole string) collapsed these to single braces because it couldn't tell a template-level
    escape from characters that came from the injected value."""
    from skit import launcher, store
    from skit.langs import launch

    entry = store.add_command("run --q {q}", name="brace-value")
    cmd = launcher.build_command(entry, values={"q": "{{ .name }}"})
    assert isinstance(cmd, str)
    # The old two-pass implementation collapsed this to "run --q { .name }" (single braces).
    # Quote with the same platform-aware helper the product uses (shlex on POSIX, list2cmdline on
    # Windows) rather than hardcoding shlex.quote, which would diverge on Windows.
    assert cmd == "run --q " + launch.quote_for_shell("{{ .name }}")


def test_placeholder_value_with_double_braces_inside_quoted_template_slot():
    """Same corruption, reproduced with the exact template shape from the finding: `echo
    {msg}` where msg embeds its own escape-like braces."""
    from skit import launcher, store

    entry = store.add_command("echo {msg}", name="brace-value-2")
    cmd = launcher.build_command(entry, values={"msg": "prefix{{inner}}suffix"})
    assert isinstance(cmd, str)
    assert "prefix{{inner}}suffix" in cmd


def test_template_escape_still_unescaped_alongside_a_corrupting_value():
    """The template's OWN {{name}} escape must still be unescaped to a literal brace, while a
    substituted value's incidental "{{"/"}}" must NOT be — proving the two are now
    distinguished (single regex pass over the original template) rather than conflated."""
    from skit import launcher, store

    entry = store.add_command("echo {{literal}} {msg}", name="brace-value-3")
    cmd = launcher.build_command(entry, values={"msg": "{{escaped-looking}}"})
    assert isinstance(cmd, str)
    assert "{literal}" in cmd  # template escape: unescaped to a single brace
    assert "{{escaped-looking}}" in cmd  # value content: left exactly as given


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell + real subprocess execution")
def test_run_entry_executes_correctly_with_double_brace_value(tmp_path):
    """End-to-end: run the assembled command for real and check the child actually received the
    value byte-for-byte, including its "{{"/"}}" — not just that build_command's string looks
    right."""
    from skit import launcher, store

    outfile = tmp_path / "out.txt"
    # {outfile} below is a Python f-string substitution (done before store ever sees the
    # template), matching the existing pattern in tests/test_exec_mut.py; {msg} is the one real
    # skit placeholder.
    entry = store.add_command(f'printf "%s" {{msg}} > "{outfile}"', name="brace-run")
    code = launcher.run_entry(entry, values={"msg": "prefix{{inner}}suffix"}, invoke_cwd=tmp_path)
    assert code == 0
    assert outfile.read_text(encoding="utf-8") == "prefix{{inner}}suffix"


# ---------- (2) signal-death exit codes must be normalized to 128+N ----------


def test_normalize_exit_code_maps_negative_returncode_to_128_plus_n():
    from skit import launcher

    assert launcher._normalize_exit_code(-11) == 139  # SIGSEGV
    assert launcher._normalize_exit_code(-15) == 143  # SIGTERM
    assert launcher._normalize_exit_code(0) == 0
    assert launcher._normalize_exit_code(2) == 2


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX signal semantics")
def test_run_entry_normalizes_signal_killed_child_to_shell_convention(tmp_path):
    """End-to-end: a command entry that kills its own shell with SIGTERM must come back as 143
    (128+15), not the raw -15 subprocess reports."""
    from skit import launcher, store

    entry = store.add_command("kill -TERM $$", name="self-signal")
    code = launcher.run_entry(entry, invoke_cwd=tmp_path)
    assert code == 143


# ---------- (3) _build_python must check the script before touching uv ----------


def test_build_python_missing_script_raises_before_calling_ensure_uv(py_entry, monkeypatch):
    """On the CLI run path (no preflight call), a missing script must be reported without ever
    resolving/downloading uv — mirrors preflight's existing ordering."""
    from skit import launcher

    def _boom():
        raise AssertionError("must not call ensure_uv before the script-exists check")

    monkeypatch.setattr("skit.langs.launch.ensure_uv", _boom)
    py_entry.script_path.unlink()
    with pytest.raises(launcher.LaunchError, match="script"):
        launcher.build_command(py_entry)


def test_build_python_healthy_script_still_calls_ensure_uv(py_entry, monkeypatch):
    """Sanity check for the reordering: when the script DOES exist, ensure_uv must still run (the
    reorder must not accidentally skip it)."""
    from skit import launcher

    calls = []
    monkeypatch.setattr("skit.langs.launch.ensure_uv", lambda: (calls.append(1), "/fake/uv")[1])
    cmd = launcher.build_command(py_entry)
    assert calls == [1]
    assert cmd[0] == "/fake/uv"


# ---------- (4) command-template placeholder values must be shell-quoted ----------


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell quoting")
def test_placeholder_value_with_space_is_quoted_as_one_word():
    from skit import launcher, store

    entry = store.add_command("ffmpeg -i {input} out.mp4", name="quote-value")
    cmd = launcher.build_command(entry, values={"input": "My Movie.mp4"})
    assert isinstance(cmd, str)
    assert cmd == "ffmpeg -i " + shlex.quote("My Movie.mp4") + " out.mp4"
    assert shlex.split(cmd) == ["ffmpeg", "-i", "My Movie.mp4", "out.mp4"]


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell quoting")
def test_placeholder_value_with_shell_metacharacters_cannot_inject():
    from skit import launcher, store

    entry = store.add_command("echo {msg}", name="quote-value-2")
    hostile = "a; rm -rf x"
    cmd = launcher.build_command(entry, values={"msg": hostile})
    assert isinstance(cmd, str)
    # Quoted as a single shell word, not parsed as a second command.
    assert shlex.split(cmd) == ["echo", hostile]


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell execution")
def test_run_entry_placeholder_value_with_space_reaches_child_intact(tmp_path):
    """End-to-end: a value with an embedded space must arrive at the child as ONE argument, not
    be split into two by the shell — reproducing the ffmpeg scenario from the finding."""
    from skit import launcher, store

    outfile = tmp_path / "out.txt"
    entry = store.add_command(f'printf "%s|%s|" {{a}} {{b}} > "{outfile}"', name="two-word-value")
    code = launcher.run_entry(
        entry, values={"a": "My Movie.mp4", "b": "second"}, invoke_cwd=tmp_path
    )
    assert code == 0
    assert outfile.read_text(encoding="utf-8") == "My Movie.mp4|second|"


def test_quote_for_shell_uses_list2cmdline_on_windows(monkeypatch):
    """The win32 branch of _quote_for_shell must use subprocess.list2cmdline (Windows quoting),
    not shlex.quote (POSIX). list2cmdline is a pure algorithm that runs on any host, so we drive
    the branch by faking the platform and assert the two quoters genuinely differ for a spaced value."""
    import subprocess

    from skit.langs import launch

    monkeypatch.setattr("sys.platform", "win32")
    value = "My Movie.mp4"
    quoted = launch.quote_for_shell(value)
    assert quoted == subprocess.list2cmdline([value])
    assert quoted == '"My Movie.mp4"'  # Windows double-quote wrapping, not POSIX single-quote
    assert quoted != shlex.quote(value)
