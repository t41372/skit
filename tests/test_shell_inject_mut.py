"""Mutation-kill tests for `langs.shell.inject` — each pins a real, observable behaviour of the
shell value-injector that a surviving mutant would break.

Companion to test_shell_inject.py (which owns the broad contract); this file targets the exact
gaps left by mutation testing. Every test runs the real injector on representative inputs — a real
tree-sitter parse, a real byte-span rewrite, and (where the claim is runtime behaviour) the real
injected copy under a real shell. English catalog, so gettext returns the msgid verbatim and
message assertions are exact.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

import pytest

from skit.langs.base import (
    InjectError,
    InjectGapError,
    InjectRequest,
    InjectSyntaxError,
    InjectValueError,
)
from skit.langs.shell import analyzer, inject
from skit.params import ParamDecl

posix_only = pytest.mark.skipif(sys.platform == "win32", reason="POSIX shells only")


def specs_of(src: str) -> list[ParamDecl]:
    return [ParamDecl.from_candidate(c) for c in analyzer.analyze(src).candidates]


def inject_src(
    src: str,
    values: dict[str, str],
    tmp_path: Path,
    *,
    specs: list[ParamDecl] | None = None,
    interpreter: str = "bash",
):
    return inject.inject(
        InjectRequest(
            text=src,
            specs=specs_of(src) if specs is None else specs,
            values=values,
            entry_dir=tmp_path,
            interpreter=interpreter,
        )
    )


def run_shell(path: Path, cwd: Path, *, shell: str = "bash", stdin: str = ""):
    return subprocess.run(
        [shell, str(path)], input=stdin, capture_output=True, text=True, cwd=cwd, check=False
    )


def temp_files(tmp_path: Path) -> list[Path]:
    return list(tmp_path.glob(".injected-*"))


# ---------------------------------------------------------------- _check_prefix


def test_gap_after_the_first_filled_variable_is_still_refused(tmp_path):
    # `read A B C` with input-1 filled, input-2 skipped, input-3 filled: the gap is at input-2,
    # AFTER a filled variable. The scanner must keep going past input-1 (a `break` there would miss
    # the gap and silently bind "z" to the wrong variable).
    src = '#!/usr/bin/env bash\nread -p "p: " A B C\n'
    with pytest.raises(InjectGapError) as exc:
        inject_src(src, {"input-1": "x", "input-3": "z"}, tmp_path)
    assert (exc.value.empty, exc.value.filled) == ("input-2", "input-3")
    assert not temp_files(tmp_path)


def test_two_empty_values_are_a_short_line_not_a_gap(tmp_path):
    # Both variables managed-empty = an empty line, which `read` accepts (both read empty). An empty
    # value is not a "filled later field", so it must NOT make the earlier empty look like a gap.
    src = '#!/usr/bin/env bash\nread -p "p: " A B\n'
    result = inject_src(src, {"input-1": "", "input-2": ""}, tmp_path)
    assert result.path is not None  # accepted, not refused
    result.path.unlink()


# ---------------------------------------------------------------- _command_name_span


@posix_only
def test_command_read_spelling_is_rewritten_whole(tmp_path):
    # `command read x` must become `_skit_read … x`, not leave a dangling `read` after the wrapper
    # (which would run the real read on garbage). The whole `command read` pair is one span.
    src = '#!/bin/sh\ncommand read -p "Name: " who\necho "hi $who"\n'
    result = inject_src(src, {"input-1": "Ada"}, tmp_path, interpreter="sh")
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    # The call site's `command read` is one span replaced whole by the wrapper — no dangling `read`
    # left after it (a partial rewrite would leave `_skit_read … read -p …` and misfeed the value).
    assert 'command read -p "Name: " who' not in text
    assert "_skit_read 0 " in text
    assert run_shell(result.path, tmp_path).stdout == "Name: Ada\nhi Ada\n"


# ---------------------------------------------------------------- _const_literal (coercion errors)


def test_const_literal_bad_int_error_carries_every_field():
    with pytest.raises(InjectValueError) as exc:
        inject._const_literal("nope", "int", "WIDTH")
    assert exc.value.value == "nope"
    assert exc.value.type_name == "int"
    assert exc.value.param_name == "WIDTH"


def test_const_literal_bad_float_error_carries_every_field():
    with pytest.raises(InjectValueError) as exc:
        inject._const_literal("nope", "float", "RATE")
    assert exc.value.value == "nope"
    assert exc.value.type_name == "float"
    assert exc.value.param_name == "RATE"


def test_const_literal_non_finite_error_carries_every_field():
    # `inf` parses as a float but is refused as not-the-number-the-user-meant; the structured fields
    # a caller reads to word its message must survive that separate refusal path too.
    with pytest.raises(InjectValueError) as exc:
        inject._const_literal("inf", "float", "RATE")
    assert exc.value.value == "inf"
    assert exc.value.type_name == "float"
    assert exc.value.param_name == "RATE"


# ---------------------------------------------------------------- _const_targets


def test_a_subscript_named_target_is_drift_never_rewritten(tmp_path):
    # `ARR[0]=1`'s name node is a `subscript`, not a `variable_name`: it is not a const candidate,
    # so a stored definition naming it is drift (InjectError), never an array-element rewrite.
    src = "#!/usr/bin/env bash\nARR[0]=1\n"
    spec = ParamDecl(name="ARR[0]", binding="const", delivery="inject", type="int")
    with pytest.raises(InjectError) as exc:
        inject_src(src, {"ARR[0]": "5"}, tmp_path, specs=[spec])
    assert "ARR[0]" in str(exc.value)
    assert not isinstance(exc.value, InjectValueError)
    assert not temp_files(tmp_path)


# ---------------------------------------------------------------- _gate_interpreter


def test_interpreter_gate_message_carries_the_shell_and_stderr_first_line(tmp_path, monkeypatch):
    class _Proc:
        returncode = 1
        stderr = b"line 3: syntax error near unexpected token\nsecond line\n"

    monkeypatch.setattr(inject.subprocess, "run", lambda *a, **k: _Proc())
    with pytest.raises(InjectSyntaxError) as exc:
        inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert str(exc.value) == (
        "bash rejected the injected copy: line 3: syntax error near unexpected token"
    )
    assert not temp_files(tmp_path)


def test_interpreter_gate_message_with_empty_stderr_has_an_empty_detail(tmp_path, monkeypatch):
    class _Proc:
        returncode = 1
        stderr = b""

    monkeypatch.setattr(inject.subprocess, "run", lambda *a, **k: _Proc())
    with pytest.raises(InjectSyntaxError) as exc:
        inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert str(exc.value) == "bash rejected the injected copy: "
    assert not temp_files(tmp_path)


def test_interpreter_gate_decodes_invalid_utf8_stderr_instead_of_crashing(tmp_path, monkeypatch):
    # A shell can emit non-UTF-8 bytes on stderr; the gate must decode with errors="replace" and
    # still raise its own InjectSyntaxError, never let a UnicodeDecodeError/LookupError escape.
    class _Proc:
        returncode = 1
        stderr = b"\xff\xfe bad token here\n"

    monkeypatch.setattr(inject.subprocess, "run", lambda *a, **k: _Proc())
    with pytest.raises(InjectSyntaxError) as exc:
        inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert "bad token here" in str(exc.value)
    assert not temp_files(tmp_path)


def test_interpreter_gate_runs_without_check_and_under_a_finite_timeout(tmp_path, monkeypatch):
    # The gate inspects the returncode itself, so it must NOT pass check=True (which would let a
    # rejected copy raise CalledProcessError, swallowed by the except and silently accepted); and it
    # must bound the run with the liveness timeout so a hung gate never wedges a launch.
    seen: dict[str, object] = {}

    class _Proc:
        returncode = 0
        stderr = b""

    def spy(argv, **kwargs):
        seen.update(kwargs)
        return _Proc()

    monkeypatch.setattr(inject.subprocess, "run", spy)
    result = inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert seen["check"] is False
    assert seen["timeout"] == inject._GATE_TIMEOUT
    assert result.path is not None
    result.path.unlink()


# ---------------------------------------------------------------- _gate_reparse


def test_offline_gate_message_is_the_corruption_notice(tmp_path, monkeypatch):
    monkeypatch.setattr(inject, "quote", lambda value: f"'{value}")  # break the escaper
    with pytest.raises(InjectSyntaxError) as exc:
        inject_src("#!/usr/bin/env bash\nTITLE=hello\n", {"TITLE": "x"}, tmp_path)
    assert str(exc.value) == (
        "the injected copy no longer parses as a shell script (nothing was run)"
    )
    assert not temp_files(tmp_path)


# ---------------------------------------------------------------- _insert_preamble / _preamble


@posix_only
def test_preamble_is_a_pure_insertion_not_a_duplicating_splice(tmp_path):
    # The preamble span is zero-width [offset, offset): a corrupted end would duplicate the tail of
    # the file. Assert the shebang and body each appear exactly once, and the copy still runs.
    src = '#!/usr/bin/env bash\nread -p "Name: " who\necho "hi $who"\n'
    result = inject_src(src, {"input-1": "Ada"}, tmp_path)
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert text.count("#!/usr/bin/env bash") == 1
    assert text.count('echo "hi $who"') == 1
    assert run_shell(result.path, tmp_path).stdout == "Name: Ada\nhi Ada\n"


def test_preamble_carries_the_shim_marker_comment(tmp_path):
    src = '#!/usr/bin/env bash\nread -p "Name: " who\n'
    result = inject_src(src, {"input-1": "Ada"}, tmp_path)
    assert result.path is not None
    assert "}  # skit:shim\n" in result.path.read_text(encoding="utf-8")
    result.path.unlink()


# ---------------------------------------------------------------- secret masking (_read_sites / _read_spans / inject)


@posix_only
def test_analyzer_detected_secret_is_masked_even_without_a_secret_spec(tmp_path):
    # `read -s` is detected by the analyzer (site.secret), so the echo is masked even when the stored
    # spec carries secret=False — the site's own certainty, not just the spec, drives the mask.
    src = '#!/usr/bin/env bash\nread -s -p "Password: " PW\necho "len=${#PW}"\n'
    spec = ParamDecl(
        name="input-1", binding="input", delivery="inject", order=0, prompt="Password: "
    )
    result = inject_src(src, {"input-1": "hunter2"}, tmp_path, specs=[spec])
    assert result.path is not None
    out = run_shell(result.path, tmp_path).stdout
    assert "Password: ***" in out  # masked via the analyzer's -s detection
    assert "hunter2" not in out
    assert "len=7" in out  # ...and the real value still delivered


@posix_only
def test_spec_marked_secret_masks_a_plain_read_via_its_order(tmp_path):
    # A plain `read` (no -s) is not secret to the analyzer, so masking relies entirely on the spec's
    # order being recorded in secret_orders. If that record is wrong, the value leaks in the echo.
    src = '#!/usr/bin/env bash\nread -p "K: " K\necho "got=$K"\n'
    spec = ParamDecl(
        name="input-1", binding="input", delivery="inject", order=0, prompt="K: ", secret=True
    )
    result = inject_src(src, {"input-1": "topsecret"}, tmp_path, specs=[spec])
    assert result.path is not None
    out = run_shell(result.path, tmp_path).stdout
    assert "K: ***" in out  # masked because the spec marked it secret
    assert "K: topsecret" not in out
    assert "got=topsecret" in out  # value still delivered to the script


# ---------------------------------------------------------------- _warnings


def test_self_location_warning_is_the_full_normalize_guidance(tmp_path):
    src = '#!/usr/bin/env bash\nHERE=$(dirname "$0")\nWIDTH=800\n'
    result = inject_src(src, {"WIDTH": "1200"}, tmp_path)
    assert result.warnings == [
        "⚠ This script reads its own location ($0 / $BASH_SOURCE), and the injected values "
        "run from a temporary copy — so it sees the copy's path, not the original's. "
        'Rewriting a constant as NAME="${NAME:-value}" delivers the value through the '
        "environment instead, with no copy at all (`skit params <script> --normalize NAME` "
        "does the rewrite for you on a stored copy)."
    ]
    assert result.path is not None
    result.path.unlink()


# ---------------------------------------------------------------- inject (write_injected / matching / drift)


def test_injected_copy_carries_the_sh_suffix(tmp_path):
    result = inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert result.path is not None
    assert result.path.suffix == ".sh"  # the interpreter recognizes the temp copy by extension
    result.path.unlink()


def test_injected_copy_falls_back_to_entry_dir_when_os_temp_is_unusable(tmp_path, monkeypatch):
    # write_injected writes to the OS temp dir, with entry_dir as the documented fallback for a
    # misconfigured TMPDIR. Force the OS-temp path to fail and the copy must land in entry_dir —
    # which it only can if inject() actually passes entry_dir through.
    real_mkstemp = tempfile.mkstemp

    def only_when_dir_given(*args, **kwargs):
        if kwargs.get("dir") is None:
            raise OSError("OS temp unwritable")
        return real_mkstemp(*args, **kwargs)

    monkeypatch.setattr(tempfile, "mkstemp", only_when_dir_given)
    result = inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert result.path is not None
    assert result.path.parent == tmp_path  # fell back to entry_dir, not raised
    result.path.unlink()


def test_only_input_specs_with_values_are_matched_to_read_sites(tmp_path):
    # The `stored` filter is `binding == "input" AND name in values`: a NON-input spec that happens
    # to carry a value (here an env-default) must never enter read-site matching and poach the site
    # its prompt collides with — which would orphan the real input spec into spurious drift.
    src = '#!/usr/bin/env bash\nread -p "P: " A\n'
    specs = [
        ParamDecl(name="E", binding="envdefault", delivery="env", order=0, prompt="P: "),
        ParamDecl(name="input-1", binding="input", delivery="inject", order=1, prompt="P: "),
    ]
    result = inject_src(src, {"E": "x", "input-1": "V1"}, tmp_path, specs=specs)
    assert result.env == {"E": "x"}
    assert result.path is not None  # input-1 kept its site; no drift
    result.path.unlink()


def test_drift_error_lists_every_missing_name_joined(tmp_path):
    # A missing input target and a missing const target must BOTH be reported: the loop keeps going
    # past the first miss, and the names are joined with ", " exactly.
    src = "#!/usr/bin/env bash\nX=1\n"
    specs = [
        ParamDecl(name="input-1", binding="input", delivery="inject", order=0, prompt="P: "),
        ParamDecl(name="GONE", binding="const", delivery="inject", type="str"),
    ]
    with pytest.raises(InjectError) as exc:
        inject_src(src, {"input-1": "v", "GONE": "x"}, tmp_path, specs=specs)
    assert str(exc.value) == "input-1, GONE"
    assert not temp_files(tmp_path)


def test_refused_copy_cleanup_survives_an_already_removed_file(tmp_path, monkeypatch):
    # On a gate-2 refusal the temp copy is removed with missing_ok=True, so that if the copy is
    # already gone the ORIGINAL refusal still surfaces — not a spurious FileNotFoundError masking it.
    def gate_removes_then_refuses(interpreter, path):
        path.unlink()  # the copy is already gone by the time inject()'s cleanup runs
        raise InjectSyntaxError("gate says no")

    monkeypatch.setattr(inject, "_gate_interpreter", gate_removes_then_refuses)
    with pytest.raises(InjectSyntaxError):
        inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert not temp_files(tmp_path)


def test_drift_error_keeps_scanning_past_a_missing_const(tmp_path):
    # Two missing const targets: the const branch must `continue` past the first, not `break` — both
    # names belong in the drift report.
    src = "#!/usr/bin/env bash\nX=1\n"
    specs = [
        ParamDecl(name="GONE1", binding="const", delivery="inject", type="str"),
        ParamDecl(name="GONE2", binding="const", delivery="inject", type="str"),
    ]
    with pytest.raises(InjectError) as exc:
        inject_src(src, {"GONE1": "a", "GONE2": "b"}, tmp_path, specs=specs)
    assert str(exc.value) == "GONE1, GONE2"
    assert not temp_files(tmp_path)
