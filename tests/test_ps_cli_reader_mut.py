"""Mutation pins for langs/powershell/cli_reader.py.

Companion to test_powershell.py — same monkeypatch-the-subprocess strategy, but these pin the
edges that the broad type/envelope suite leaves mutation-alive: the exact pwsh command line and
the ``check`` guard skit hands to ``subprocess.run``, the utf-8/errors decode contract on the
process output, the empty-help and empty-ValidateSet fall-throughs in the row mapper, and the
``.ps1`` suffix of the temp file skit parses. Each test exercises the real ``read_cli`` path with
no pwsh installed (the extractor never runs); the assertions are the observable call contract.
"""

from __future__ import annotations

import json
import subprocess

from skit.langs.powershell import cli_reader

# ---------------------------------------------------------------- helpers (mirror test_powershell)


def _only_pwsh(monkeypatch):
    """Make read_cli believe pwsh (and only pwsh) is on PATH."""
    monkeypatch.setattr(
        cli_reader.shutil, "which", lambda name: "/usr/bin/pwsh" if name == "pwsh" else None
    )


def _row(name, static="System.String", **over):
    row = {
        "name": name,
        "staticType": static,
        "switch": False,
        "hasDefault": False,
        "defaultReadable": False,
        "defaultConst": None,
        "mandatory": False,
        "validateSet": None,
        "helpText": None,
    }
    row.update(over)
    return row


def _read_rows(monkeypatch, rows, *, stdout=None):
    """Drive read_cli with a stubbed subprocess returning the given rows (or raw stdout)."""
    _only_pwsh(monkeypatch)
    if stdout is None:
        stdout = json.dumps({"status": "ok", "params": rows}).encode("utf-8")

    def fake_run(argv, **kwargs):
        return subprocess.CompletedProcess(argv, 0, stdout=stdout, stderr=b"")

    monkeypatch.setattr(cli_reader.subprocess, "run", fake_run)
    spec = cli_reader.read_cli("param()\n")
    assert spec is not None
    return {f.name: f for f in spec.fields}


# ---------------------------------------------------------------- the pwsh command contract


def test_read_cli_invokes_pwsh_with_the_documented_argv_and_check_guard(monkeypatch):
    # Nothing else pins the argv or the check flag: the type suite's fake ignores both, so a
    # mutant that lower-cased a flag, retitled -Command, or flipped check would sail through.
    # Capture the real subprocess.run call and assert the invocation contract.
    _only_pwsh(monkeypatch)
    seen = {}

    def capturing_run(argv, **kwargs):
        seen["argv"] = argv
        seen.update(kwargs)
        return subprocess.CompletedProcess(argv, 0, stdout=b'{"status":"no-params"}', stderr=b"")

    monkeypatch.setattr(cli_reader.subprocess, "run", capturing_run)
    cli_reader.read_cli("param()\n")

    assert seen["argv"] == [
        "/usr/bin/pwsh",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        cli_reader._EXTRACTOR,
    ]
    # check=False: skit inspects returncode itself and must NOT let run() raise on non-zero exit.
    assert seen["check"] is False


def test_non_utf8_stdout_degrades_to_none_instead_of_raising(monkeypatch):
    # The extractor's bytes are decoded with errors="replace" so a stray non-utf-8 byte becomes a
    # replacement char (unparseable JSON -> None), never an exception that escapes read_cli. A
    # strict decode (dropped errors=) or a bogus handler name would raise UnicodeDecodeError /
    # LookupError, which read_cli does NOT catch -> the call would blow up instead of degrading.
    bad = b"\xff\xfe not valid utf-8"
    _only_pwsh(monkeypatch)

    def fake_run(argv, **kwargs):
        return subprocess.CompletedProcess(argv, 0, stdout=bad, stderr=b"")

    monkeypatch.setattr(cli_reader.subprocess, "run", fake_run)
    assert cli_reader.read_cli("param()\n") is None


def test_missing_help_text_yields_empty_help_not_a_placeholder(monkeypatch):
    # A row with no comment-based help must produce help="" (the `or ""` fallback), not some
    # non-empty sentinel — an empty string is how the form knows there is no help to show.
    fields = _read_rows(monkeypatch, [_row("Name", helpText=None)])
    assert fields["Name"].help == ""


def test_empty_validate_set_falls_through_to_the_static_type(monkeypatch):
    # `[ValidateSet()]` with no members is not a choice constraint: an empty list must fall
    # through to the static-type mapping (System.String -> str), not be treated as a choice.
    fields = _read_rows(monkeypatch, [_row("Mode", static="System.String", validateSet=[])])
    f = fields["Mode"]
    assert f.type == "str"
    assert f.choices == ()


def test_read_cli_writes_a_dot_ps1_temp_file(monkeypatch):
    # skit hands pwsh a real file on disk to parse; it names it *.ps1. Capture the suffix passed
    # to mkstemp (delegating to the real mkstemp so the rest of read_cli runs unchanged).
    _only_pwsh(monkeypatch)
    real_mkstemp = cli_reader.tempfile.mkstemp
    seen = {}

    def capturing_mkstemp(*args, **kwargs):
        seen["suffix"] = kwargs.get("suffix", args[0] if args else None)
        return real_mkstemp(*args, **kwargs)

    monkeypatch.setattr(cli_reader.tempfile, "mkstemp", capturing_mkstemp)
    monkeypatch.setattr(
        cli_reader.subprocess,
        "run",
        lambda argv, **k: subprocess.CompletedProcess(
            argv, 0, stdout=b'{"status":"no-params"}', stderr=b""
        ),
    )
    cli_reader.read_cli("param()\n")
    assert seen["suffix"] == ".ps1"
