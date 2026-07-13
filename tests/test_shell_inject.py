"""Shell injector contract: the three deliveries, the two syntax gates, and the correctness risks.

The execution tests are the point of this file. A shim that *looks* right in a diff is worth
nothing — every claim here (the value lands, the prompt echoes, the secret is masked, the payload
is inert, the second loop iteration reads real stdin) is proven by RUNNING the injected copy under
a real shell and asserting on the child's own output. They are POSIX-only (skipped on Windows,
where the shell kind's `bash` may not exist at all); the pure-logic tests below them run everywhere.

Risk coverage (docs/design/multilang.md, "Top correctness risks"):
  #2 wrong call site  -> test_function_read_defined_above_invoked_after_keeps_its_value
  #3 quoting injection -> test_const_payload_is_inert / test_read_payload_is_inert
  #4 multibyte / CRLF  -> test_cjk_emoji_* / test_crlf_*
  #7 double binding    -> test_two_specs_claiming_one_read_site_is_drift
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, flows, store
from skit.langs.base import (
    InjectError,
    InjectGapError,
    InjectRequest,
    InjectSplitError,
    InjectSyntaxError,
    InjectValueError,
)
from skit.langs.shell import analyzer, inject, normalize
from skit.params import ParamDecl

runner = CliRunner()

posix_only = pytest.mark.skipif(sys.platform == "win32", reason="POSIX shells only")

# The dialects the preamble claims to support. Each is skipped when it isn't installed, so the
# suite still passes on a bare CI image — but wherever the shell exists, the claim is proven.
SHELLS = ["bash", "sh", "zsh", "dash"]


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


def run_shell(
    path: Path, cwd: Path, *, shell: str = "bash", stdin: str = ""
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [shell, str(path)], input=stdin, capture_output=True, text=True, cwd=cwd, check=False
    )


def temp_files(tmp_path: Path) -> list[Path]:
    """Any injected copy that fell back into entry_dir (the OS temp dir is used by default)."""
    return list(tmp_path.glob(".injected-*"))


def read_raw(path: Path) -> str:
    """The file's bytes as-is: newline="" so a CRLF script's line endings survive the read (text
    mode would silently translate them, hiding exactly the bug the CRLF test is looking for)."""
    with path.open(encoding="utf-8", newline="") as f:
        return f.read()


# ---------------------------------------------------------------- const delivery


@posix_only
def test_const_injection_runs_with_the_new_value(tmp_path):
    src = '#!/usr/bin/env bash\nWIDTH=800\necho "w=$WIDTH"\n'
    result = inject_src(src, {"WIDTH": "1200"}, tmp_path)
    assert result.path is not None
    assert run_shell(result.path, tmp_path).stdout == "w=1200\n"


@posix_only
def test_const_str_is_single_quoted_and_int_is_bare(tmp_path):
    src = "#!/usr/bin/env bash\nWIDTH=800\nCITY=Taipei\n"
    result = inject_src(src, {"WIDTH": "1200", "CITY": "New York"}, tmp_path)
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert "WIDTH=1200" in text  # int coerces -> bare word, no quotes
    assert "CITY='New York'" in text  # str -> POSIX single-quoted, always


@posix_only
def test_const_rewrites_every_same_name_occurrence(tmp_path):
    src = '#!/usr/bin/env bash\nMODE=fast\nMODE=slow\necho "$MODE"\n'
    result = inject_src(src, {"MODE": "turbo"}, tmp_path)
    assert result.path is not None
    assert result.path.read_text(encoding="utf-8").count("MODE='turbo'") == 2
    assert run_shell(result.path, tmp_path).stdout == "turbo\n"


@posix_only
def test_const_quoting_is_normalized_not_preserved(tmp_path):
    # The source quoting (raw string, double-quoted, bare word) is irrelevant: every str value
    # comes out single-quoted, which is what makes injection impossible.
    src = "#!/usr/bin/env bash\nA=bare\nB='raw'\nC=\"double\"\n"
    result = inject_src(src, {"A": "x y", "B": "x y", "C": "x y"}, tmp_path)
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert text.count("='x y'") == 3


def test_bad_int_value_raises_the_value_error_not_drift(tmp_path):
    src = "#!/usr/bin/env bash\nWIDTH=800\n"
    with pytest.raises(InjectValueError) as exc_info:
        inject_src(src, {"WIDTH": "not-a-number"}, tmp_path)
    assert exc_info.value.param_name == "WIDTH"
    assert not temp_files(tmp_path)  # nothing was written


def test_bad_float_and_non_finite_values_are_refused(tmp_path):
    src = "#!/usr/bin/env bash\nRATE=0.5\n"
    for bad in ("abc", "inf", "-inf", "nan"):
        with pytest.raises(InjectValueError):
            inject_src(src, {"RATE": bad}, tmp_path)


@posix_only
def test_float_const_injects_a_bare_number(tmp_path):
    src = '#!/usr/bin/env bash\nRATE=0.5\necho "r=$RATE"\n'
    result = inject_src(src, {"RATE": "2.75"}, tmp_path)
    assert result.path is not None
    assert run_shell(result.path, tmp_path).stdout == "r=2.75\n"


def test_missing_const_target_is_drift(tmp_path):
    src = "#!/usr/bin/env bash\nWIDTH=800\n"
    spec = ParamDecl(name="GONE", binding="const", delivery="inject", type="str")
    with pytest.raises(InjectError) as exc_info:
        inject_src(src, {"GONE": "x"}, tmp_path, specs=[spec])
    assert "GONE" in str(exc_info.value)
    assert not isinstance(exc_info.value, InjectValueError)
    assert not temp_files(tmp_path)


def test_readonly_const_is_never_a_target(tmp_path):
    # The analyzer never offers a readonly const, so a stored definition naming one is drift —
    # rewriting it would produce a script that dies with "readonly variable" at run time.
    src = "#!/usr/bin/env bash\nreadonly MAX=100\n"
    spec = ParamDecl(name="MAX", binding="const", delivery="inject", type="int")
    with pytest.raises(InjectError):
        inject_src(src, {"MAX": "5"}, tmp_path, specs=[spec])


@posix_only
def test_const_targets_skip_array_and_valueless_assignments(tmp_path):
    # `ARR[0]=…` (a subscript target) and `EMPTY=` (no value) are not const candidates, so they are
    # not rewrite targets either — the two sides must agree, or a run would rewrite what the form
    # never offered.
    src = '#!/usr/bin/env bash\nARR[0]=1\nEMPTY=\nWIDTH=800\necho "$WIDTH${ARR[0]}[$EMPTY]"\n'
    result = inject_src(src, {"WIDTH": "1200"}, tmp_path)
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert "ARR[0]=1" in text
    assert "EMPTY=\n" in text
    assert run_shell(result.path, tmp_path).stdout == "12001[]\n"


def test_no_values_writes_nothing_at_all(tmp_path):
    src = "#!/usr/bin/env bash\nWIDTH=800\nread -p 'Name: ' who\n"
    result = inject_src(src, {}, tmp_path)
    assert result.path is None  # no temp copy, no rewrite: the original file runs
    assert result.env == {}
    assert not temp_files(tmp_path)


# ---------------------------------------------------------------- env delivery


def test_env_delivery_writes_no_temp_file(tmp_path):
    src = '#!/usr/bin/env bash\necho "${GREETING:-hello}"\n'
    result = inject_src(src, {"GREETING": "hi there"}, tmp_path)
    assert result.path is None  # THE point of env delivery: zero rewrite, no temp copy, $0 intact
    assert result.env == {"GREETING": "hi there"}
    assert result.warnings == []
    assert not temp_files(tmp_path)


@posix_only
def test_env_delivery_actually_reaches_the_script(tmp_path):
    src = tmp_path / "s.sh"
    src.write_text('#!/usr/bin/env bash\necho "${GREETING:-hello}"\n', encoding="utf-8")
    proc = subprocess.run(
        ["bash", str(src)],  # noqa: S607 — bash from PATH is exactly what the shell kind runs
        capture_output=True,
        text=True,
        env={"PATH": "/usr/bin:/bin", "GREETING": "hi there"},
        check=False,
    )
    assert proc.stdout == "hi there\n"


@posix_only
def test_mixed_env_and_const_delivery(tmp_path):
    src = '#!/usr/bin/env bash\nWIDTH=800\necho "${MODE:-auto} $WIDTH"\n'
    result = inject_src(src, {"WIDTH": "1200", "MODE": "manual"}, tmp_path)
    assert result.env == {"MODE": "manual"}  # the envdefault never touches the source
    assert result.path is not None  # ...but the const still needs its temp copy
    assert "WIDTH=1200" in result.path.read_text(encoding="utf-8")


# ---------------------------------------------------------------- read delivery


@posix_only
def test_read_interception_echoes_prompt_and_value(tmp_path):
    src = '#!/usr/bin/env bash\nread -p "Name: " who\necho "hi $who"\n'
    result = inject_src(src, {"input-1": "Ada"}, tmp_path)
    assert result.path is not None
    assert run_shell(result.path, tmp_path).stdout == "Name: Ada\nhi Ada\n"


@posix_only
def test_read_rewrite_keeps_every_flag_and_varname(tmp_path):
    src = '#!/usr/bin/env bash\nread -r -p "Name: " who\n'
    result = inject_src(src, {"input-1": "Ada"}, tmp_path)
    assert result.path is not None
    assert "_skit_read 0 'Ada' 0 'Name: ' -r -p \"Name: \" who" in result.path.read_text(
        encoding="utf-8"
    )


@posix_only
def test_secret_read_masks_the_echo_but_delivers_the_value(tmp_path):
    src = '#!/usr/bin/env bash\nread -s -p "Password: " PW\necho "len=${#PW}"\n'
    result = inject_src(src, {"input-1": "hunter2"}, tmp_path)
    assert result.path is not None
    out = run_shell(result.path, tmp_path).stdout
    assert out == "Password: ***\nlen=7\n"  # masked echo, real length
    assert "hunter2" not in out


@posix_only
def test_read_in_a_loop_takes_the_value_once_then_reads_real_stdin(tmp_path):
    src = (
        '#!/usr/bin/env bash\nfor i in 1 2 3; do\n  read -p "Item: " it\n  echo "item=$it"\ndone\n'
    )
    result = inject_src(src, {"input-1": "first"}, tmp_path)
    assert result.path is not None
    out = run_shell(result.path, tmp_path, stdin="second\nthird\n").stdout
    assert out == "Item: first\nitem=first\nitem=second\nitem=third\n"


@posix_only
def test_function_read_defined_above_invoked_after_keeps_its_value(tmp_path):
    """Risk #2, the one that makes call-site binding non-negotiable: the function's read is FIRST
    in source order but runs LAST. A runtime counter would swap the two values (and hand the
    secret to the wrong question); binding to the call site cannot."""
    src = (
        "#!/usr/bin/env bash\n"
        "ask_secret() {\n"
        '  read -s -p "Password: " PW\n'
        "}\n"
        'read -p "Name: " NAME\n'
        "ask_secret\n"
        'echo "name=$NAME pw=$PW"\n'
    )
    values = {"input-1": "SUPERSECRET", "input-2": "alice"}  # input-1 IS the function's read
    result = inject_src(src, values, tmp_path)
    assert result.path is not None
    out = run_shell(result.path, tmp_path).stdout
    assert "name=alice pw=SUPERSECRET" in out
    assert "Password: ***" in out  # the secret's echo is masked...
    assert "Name: alice" in out  # ...and the plain one is not


def test_two_specs_claiming_one_read_site_is_drift(tmp_path):
    """Risk #7: two definitions carrying the same order (a hand-edited block) would otherwise
    splice two replacements over one command-name span, corrupting the copy into unparsable text."""
    src = '#!/usr/bin/env bash\nread -p "Go? " a\n'
    specs = [
        ParamDecl(name="input-1", binding="input", delivery="inject", order=0, prompt="Go? "),
        ParamDecl(name="input-2", binding="input", delivery="inject", order=0, prompt="Go? "),
    ]
    with pytest.raises(InjectError) as exc_info:
        inject_src(src, {"input-1": "AAA", "input-2": "BBB"}, tmp_path, specs=specs)
    assert "input-2" in str(exc_info.value)
    assert not temp_files(tmp_path)  # nothing was written


def test_vanished_read_site_is_drift(tmp_path):
    src = '#!/usr/bin/env bash\nread -p "Go? " a\n'
    spec = ParamDecl(name="input-3", binding="input", delivery="inject", order=2, prompt="Gone? ")
    with pytest.raises(InjectError):
        inject_src(src, {"input-3": "x"}, tmp_path, specs=[spec])


@posix_only
def test_value_follows_its_prompt_not_its_position(tmp_path):
    """A new read inserted ABOVE an existing one shifts every position; the stored value must
    still land on its own question (shared callmatch — the same rule reconcile uses)."""
    stored = [
        ParamDecl(
            name="input-1",
            binding="input",
            delivery="inject",
            order=0,
            prompt="Password: ",
            secret=True,
        )
    ]
    edited = (
        "#!/usr/bin/env bash\n"
        'read -p "Name: " NAME\n'  # a new read, inserted first
        'read -s -p "Password: " PW\n'
        'echo "pw=$PW name=[$NAME]"\n'
    )
    result = inject_src(edited, {"input-1": "hunter2"}, tmp_path, specs=stored)
    assert result.path is not None
    out = run_shell(result.path, tmp_path, stdin="typed\n").stdout
    assert "pw=hunter2 name=[typed]" in out  # the value followed its prompt, not position 0


@posix_only
def test_multi_variable_read_joins_its_values_on_one_line(tmp_path):
    src = '#!/usr/bin/env bash\nread -p "First and last: " FIRST LAST\necho "[$FIRST][$LAST]"\n'
    result = inject_src(src, {"input-1": "Ada", "input-2": "Lovelace"}, tmp_path)
    assert result.path is not None
    out = run_shell(result.path, tmp_path).stdout
    assert out == "First and last: Ada Lovelace\n[Ada][Lovelace]\n"


@posix_only
def test_multi_variable_read_accepts_a_short_prefix(tmp_path):
    # Only the first variable filled: exactly what a short typed line does (the rest read empty).
    src = '#!/usr/bin/env bash\nread -p "First and last: " FIRST LAST\necho "[$FIRST][$LAST]"\n'
    result = inject_src(src, {"input-1": "Ada"}, tmp_path)
    assert result.path is not None
    assert run_shell(result.path, tmp_path).stdout == "First and last: Ada\n[Ada][]\n"


def test_multi_variable_read_refuses_a_positional_gap(tmp_path):
    # input-1 empty + input-2 filled: one `read` line cannot express that — the shell would hand
    # "Lovelace" to FIRST. Refused loudly instead of binding the value to the wrong variable.
    src = '#!/usr/bin/env bash\nread -p "First and last: " FIRST LAST\n'
    with pytest.raises(InjectGapError) as exc_info:
        inject_src(src, {"input-2": "Lovelace"}, tmp_path)
    assert (exc_info.value.empty, exc_info.value.filled) == ("input-1", "input-2")
    assert not temp_files(tmp_path)


def test_multi_variable_read_refuses_whitespace_in_a_non_last_field(tmp_path):
    # "John Paul" in FIRST would spill across the IFS boundary when the joined line is re-split
    # ("John" → FIRST, "Paul" → LAST). Refused instead of silently delivering the wrong value.
    src = '#!/usr/bin/env bash\nread -p "First and last: " FIRST LAST\n'
    with pytest.raises(InjectSplitError) as exc_info:
        inject_src(src, {"input-1": "John Paul", "input-2": "Doe"}, tmp_path)
    assert exc_info.value.name == "input-1"
    assert not temp_files(tmp_path)


def test_read_refuses_a_newline_in_any_field_including_a_single_variable(tmp_path):
    # A newline ENDS the read's line, so no variable can hold it — not even the only one. Accepting
    # it delivered "a" while skit's own echo showed "a\nb": the value and the echo disagreed.
    single = '#!/usr/bin/env bash\nread -p "Name: " NAME\n'
    with pytest.raises(InjectSplitError) as exc_info:
        inject_src(single, {"input-1": "a\nb"}, tmp_path)
    assert exc_info.value.reason == "line-break"
    # and in the LAST variable of a multi-var read, which is exempt from field-splitting only
    multi = '#!/usr/bin/env bash\nread -p "A B: " A B\n'
    with pytest.raises(InjectSplitError) as exc_info:
        inject_src(multi, {"input-1": "x", "input-2": "a\nb"}, tmp_path)
    assert exc_info.value.reason == "line-break"
    assert not temp_files(tmp_path)


def test_read_refuses_edge_whitespace_that_the_shell_would_strip(tmp_path):
    # `read` strips leading/trailing IFS whitespace off the line, so " lead" would arrive as "lead"
    # — a silent modification. Interior spaces in the last variable are fine (the line's remainder).
    src = '#!/usr/bin/env bash\nread -p "Name: " NAME\n'
    for edge in (" lead", "trail ", "\ttab-lead"):
        with pytest.raises(InjectSplitError) as exc_info:
            inject_src(src, {"input-1": edge}, tmp_path)
        assert exc_info.value.reason == "edge-space"
    result = inject_src(src, {"input-1": "de Lovelace"}, tmp_path)  # interior: accepted
    assert result.path is not None


@posix_only
def test_read_accepts_a_carriage_return_which_the_shell_delivers_intact(tmp_path):
    # CR is neither a default-$IFS splitter nor a line terminator: every supported shell hands the
    # value over byte-intact, so refusing it would be a false positive.
    src = '#!/usr/bin/env bash\nread -p "V: " V\nprintf "<%s>" "$V"\n'
    result = inject_src(src, {"input-1": "a\rb"}, tmp_path)
    assert result.path is not None
    # Raw bytes on purpose: text mode would translate the CR away and hide what actually arrived.
    out = subprocess.run(
        [shutil.which("bash") or "bash", str(result.path)],
        capture_output=True,
        cwd=tmp_path,
        check=False,
    )
    assert b"<a\rb>" in out.stdout


def test_multi_variable_read_refuses_whitespace_when_a_trailing_var_is_unmanaged(tmp_path):
    # The exemption is keyed on the read's last VARIABLE, not the last supplied value: with only
    # input-1 managed, the shell still binds LAST from the same line, so "John Paul" would have
    # silently delivered FIRST="John", LAST="Paul". Refused.
    src = '#!/usr/bin/env bash\nread -p "First and last: " FIRST LAST\n'
    with pytest.raises(InjectSplitError) as exc_info:
        inject_src(src, {"input-1": "John Paul"}, tmp_path)
    assert exc_info.value.name == "input-1"
    assert not temp_files(tmp_path)


def test_multi_variable_read_refuses_a_newline_in_a_non_last_field(tmp_path):
    # The worst case Review A caught: a newline in an earlier value truncates the whole line,
    # silently discarding EVERY later field. Must be refused, not silently run.
    src = '#!/usr/bin/env bash\nread -p "First and last: " FIRST LAST\n'
    with pytest.raises(InjectSplitError):
        inject_src(src, {"input-1": "a\nb", "input-2": "KEEP"}, tmp_path)
    assert not temp_files(tmp_path)


@posix_only
def test_multi_variable_read_allows_whitespace_in_the_last_field(tmp_path):
    # The LAST variable absorbs the remainder of the line, so it may safely hold spaces —
    # exactly what a typed multi-word tail does.
    src = '#!/usr/bin/env bash\nread -p "First and last: " FIRST LAST\necho "[$FIRST][$LAST]"\n'
    result = inject_src(src, {"input-1": "Ada", "input-2": "de Lovelace"}, tmp_path)
    assert result.path is not None
    assert run_shell(result.path, tmp_path).stdout.endswith("[Ada][de Lovelace]\n")


@posix_only
def test_execute_reports_a_whitespace_split_as_a_bad_value(tmp_path):
    _shell_entry(
        tmp_path,
        '#!/usr/bin/env bash\nread -p "First and last: " FIRST LAST\necho "$FIRST $LAST"\n',
        name="exsh5",
    )
    assert (
        runner.invoke(
            cli.app, ["params", "exsh5", "--manage", "input-1", "--manage", "input-2"]
        ).exit_code
        == 0
    )
    result = runner.invoke(
        cli.app,
        ["run", "exsh5", "--set", "input-1=John Paul", "--set", "input-2=Doe", "--no-input"],
    )
    assert result.exit_code == flows.FAILURE_EXIT_CODES[flows.FAIL_BAD_VALUE]
    assert "input-1" in result.output


@posix_only
def test_builtin_read_spelling_is_rewritten_whole(tmp_path):
    # `builtin read x` must become `_skit_read …  x`, not `builtin _skit_read …` (which would try
    # to run the wrapper function as a shell builtin and fail).
    src = '#!/usr/bin/env bash\nbuiltin read -p "Name: " who\necho "hi $who"\n'
    result = inject_src(src, {"input-1": "Ada"}, tmp_path)
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert "builtin _skit_read" not in text
    assert run_shell(result.path, tmp_path).stdout == "Name: Ada\nhi Ada\n"


@posix_only
def test_unmanaged_read_still_reads_real_stdin(tmp_path):
    src = '#!/usr/bin/env bash\nread -p "One: " a\nread -p "Two: " b\necho "[$a][$b]"\n'
    result = inject_src(src, {"input-1": "injected"}, tmp_path)
    assert result.path is not None
    assert run_shell(result.path, tmp_path, stdin="typed\n").stdout.endswith("[injected][typed]\n")


@posix_only
@pytest.mark.parametrize("shell", SHELLS)
def test_the_preamble_runs_on_every_supported_dialect(tmp_path, shell):
    """bash 3.2 (macOS), zsh, sh and dash all run the wrapper verbatim — no associative arrays, no
    `[[ -v ]]`, and the fall-through keyword picked per dialect (`command read` is a silent no-op
    in zsh; dash has no `builtin` at all)."""
    if shutil.which(shell) is None:  # pragma: no cover — depends on the host's installed shells
        pytest.skip(f"{shell} not installed")
    src = '#!/bin/sh\nNAME=x\nread who\necho "hi $who / $NAME"\nread it\necho "it=$it"\n'
    result = inject_src(src, {"NAME": "y", "input-1": "Ada"}, tmp_path, interpreter=shell)
    assert result.path is not None
    proc = run_shell(result.path, tmp_path, shell=shell, stdin="typed\n")
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout == "Ada\nhi Ada / y\nit=typed\n"


@posix_only
def test_set_u_and_set_e_survive_the_preamble(tmp_path):
    src = (
        "#!/usr/bin/env bash\n"
        "set -euo pipefail\n"
        "OUT=/tmp/out\n"
        'read -p "Deploy? " confirm\n'
        'echo "$OUT $confirm"\n'
    )
    result = inject_src(src, {"OUT": "/tmp/x", "input-1": "yes"}, tmp_path)
    assert result.path is not None
    proc = run_shell(result.path, tmp_path)
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout.endswith("/tmp/x yes\n")


# ---------------------------------------------------------------- risk #3: quoting injection


PAYLOADS = ["'; touch pwned; echo '", "$(touch pwned)", "`touch pwned`", "$(id) && touch pwned"]


@posix_only
@pytest.mark.parametrize("payload", PAYLOADS)
def test_const_payload_is_inert(tmp_path, payload):
    src = '#!/usr/bin/env bash\nTITLE=hello\necho "[$TITLE]"\n'
    result = inject_src(src, {"TITLE": payload}, tmp_path)
    assert result.path is not None
    proc = subprocess.run(
        ["bash", "-n", str(result.path)],  # noqa: S607 — bash -n must accept the injected copy
        capture_output=True,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    out = run_shell(result.path, tmp_path).stdout
    assert out == f"[{payload}]\n"  # the variable holds the payload as literal text
    assert not (tmp_path / "pwned").exists()  # ...and nothing executed


@posix_only
@pytest.mark.parametrize("payload", PAYLOADS)
def test_read_payload_is_inert(tmp_path, payload):
    src = '#!/usr/bin/env bash\nread -p "Name: " who\necho "[$who]"\n'
    result = inject_src(src, {"input-1": payload}, tmp_path)
    assert result.path is not None
    out = run_shell(result.path, tmp_path).stdout
    assert out == f"Name: {payload}\n[{payload}]\n"
    assert not (tmp_path / "pwned").exists()


@posix_only
def test_quote_in_a_read_prompt_survives(tmp_path):
    # The PROMPT is re-emitted as an argument, so it goes through the same escaper: an apostrophe
    # in the script's own prompt text must not break out of the single-quoted argument.
    src = '#!/usr/bin/env bash\nread -p "It\'s here: " who\necho "[$who]"\n'
    result = inject_src(src, {"input-1": "x"}, tmp_path)
    assert result.path is not None
    assert run_shell(result.path, tmp_path).stdout == "It's here: x\n[x]\n"


@posix_only
def test_secret_value_never_reaches_stdout(tmp_path):
    src = '#!/usr/bin/env bash\nAPI_KEY=changeme\necho "done"\n'
    result = inject_src(src, {"API_KEY": "s3cr3t"}, tmp_path)
    assert result.path is not None
    # The value IS in the temp copy (it has to be), but the copy is 0600 and short-lived.
    assert result.path.stat().st_mode & 0o777 == 0o600
    assert run_shell(result.path, tmp_path).stdout == "done\n"


# ---------------------------------------------------------------- risk #4: multibyte / CRLF


@posix_only
def test_cjk_emoji_const_and_prompt_round_trip(tmp_path):
    src = '#!/usr/bin/env bash\nCITY=台北\nread -p "请输入名字 🙂: " NAME\necho "$CITY|$NAME"\n'
    result = inject_src(src, {"CITY": "高雄 🚀", "input-1": "愛達"}, tmp_path)
    assert result.path is not None
    assert not analyzer.analyze(result.path.read_text(encoding="utf-8")).syntax_error
    out = run_shell(result.path, tmp_path).stdout
    assert out == "请输入名字 🙂: 愛達\n高雄 🚀|愛達\n"


@posix_only
def test_crlf_script_injects_and_runs(tmp_path):
    src = '#!/usr/bin/env bash\r\nWIDTH=800\r\nHEIGHT=600\r\necho "$WIDTH"\r\n'
    result = inject_src(src, {"WIDTH": "1200"}, tmp_path)
    assert result.path is not None
    text = read_raw(result.path)
    assert "WIDTH=1200\r\n" in text  # the line ending survived; only the value's bytes changed
    assert not analyzer.analyze(text).syntax_error
    proc = run_shell(result.path, tmp_path)
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout.startswith("1200")


@posix_only
def test_no_trailing_newline_script_injects(tmp_path):
    result = inject_src("#!/usr/bin/env bash\nVERSION=1.2.0", {"VERSION": "2.0.0"}, tmp_path)
    assert result.path is not None
    assert result.path.read_text(encoding="utf-8").endswith("VERSION='2.0.0'")


@posix_only
def test_no_shebang_puts_the_preamble_at_the_very_top(tmp_path):
    src = 'read -p "Name: " who\necho "hi $who"\n'
    result = inject_src(src, {"input-1": "Ada"}, tmp_path)
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert text.startswith("_skit_read() {")
    assert run_shell(result.path, tmp_path).stdout == "Name: Ada\nhi Ada\n"


@posix_only
def test_preamble_lands_after_the_shebang(tmp_path):
    src = '#!/usr/bin/env bash\nread -p "Name: " who\n'
    result = inject_src(src, {"input-1": "Ada"}, tmp_path)
    assert result.path is not None
    lines = result.path.read_text(encoding="utf-8").splitlines()
    assert lines[0] == "#!/usr/bin/env bash"
    assert lines[1] == "_skit_read() {"


# ---------------------------------------------------------------- documented dialect variance


@posix_only
def test_backslash_value_follows_the_shells_own_read_rules(tmp_path):
    """DOCUMENTED VARIANCE, not a bug: skit delivers the bytes; the script's own `read` decides
    what a backslash means. `read -r` keeps it, a bare `read` eats it — exactly what the user
    typing that value at the prompt would have got. A const, by contrast, is exact everywhere."""
    raw = '#!/usr/bin/env bash\nread -r -p "P: " a\necho "[$a]"\n'
    cooked = '#!/usr/bin/env bash\nread -p "P: " a\necho "[$a]"\n'
    value = "a\\b"
    raw_out = run_shell(inject_src(raw, {"input-1": value}, tmp_path).path, tmp_path).stdout
    cooked_out = run_shell(inject_src(cooked, {"input-1": value}, tmp_path).path, tmp_path).stdout
    assert raw_out.endswith("[a\\b]\n")  # -r: the backslash survives
    assert cooked_out.endswith("[ab]\n")  # no -r: bash's read consumes it (the shell's rule)
    # A const value is not read()'s business at all, so it is byte-exact in every dialect:
    const = '#!/usr/bin/env bash\nP=x\necho "[$P]"\n'
    assert run_shell(inject_src(const, {"P": value}, tmp_path).path, tmp_path).stdout == "[a\\b]\n"


def test_fallthrough_keyword_is_dialect_selected():
    # `command read` is a silent no-op in zsh; `builtin` does not exist in dash. There is no
    # single keyword that works everywhere — so the dialect picks it.
    assert inject._fallthrough_keyword("bash") == "builtin"
    assert inject._fallthrough_keyword("zsh") == "builtin"
    assert inject._fallthrough_keyword("/bin/zsh") == "builtin"
    assert inject._fallthrough_keyword("bash.exe") == "builtin"
    assert inject._fallthrough_keyword("sh") == "command"
    assert inject._fallthrough_keyword("dash") == "command"
    assert inject._fallthrough_keyword("ksh") == "command"
    assert inject._fallthrough_keyword("") == "command"


# ---------------------------------------------------------------- the two syntax gates


def test_offline_gate_refuses_a_corrupted_injection(tmp_path, monkeypatch):
    """The has_error gate is the last line of defense: break the escaper (as a quoting bug would)
    and the injected copy must never be launched — or even survive on disk."""
    monkeypatch.setattr(inject, "quote", lambda value: f"'{value}")  # unterminated quote
    src = "#!/usr/bin/env bash\nTITLE=hello\n"
    with pytest.raises(InjectSyntaxError):
        inject_src(src, {"TITLE": "x"}, tmp_path)
    assert not temp_files(tmp_path)


@posix_only
def test_interpreter_gate_refuses_what_the_offline_gate_missed(tmp_path, monkeypatch):
    # Gate 2 is hardening: pretend the offline re-parse passed (tree-sitter is more forgiving than
    # bash in places) and prove `bash -n` still stops the launch, leaving no temp file behind.
    monkeypatch.setattr(inject, "_gate_reparse", lambda out: None)
    monkeypatch.setattr(inject, "quote", lambda value: f"'{value}")
    src = "#!/usr/bin/env bash\nTITLE=hello\n"
    with pytest.raises(InjectSyntaxError) as exc_info:
        inject_src(src, {"TITLE": "x"}, tmp_path)
    assert "bash" in str(exc_info.value)
    assert not temp_files(tmp_path)


def test_interpreter_gate_is_skipped_when_the_shell_is_not_installed(tmp_path):
    # Preflight owns "your interpreter isn't installed"; the injector must not turn that into a
    # confusing injection failure. The offline gate already vouched for the text.
    result = inject_src(
        "#!/usr/bin/env bash\nWIDTH=800\n",
        {"WIDTH": "1200"},
        tmp_path,
        interpreter="skit-no-such-shell",
    )
    assert result.path is not None
    result.path.unlink()


def test_interpreter_gate_survives_a_spawn_failure(tmp_path, monkeypatch):
    def boom(*_args, **_kwargs):
        raise OSError("no fork for you")

    monkeypatch.setattr(inject.subprocess, "run", boom)
    result = inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert (
        result.path is not None
    )  # the gate couldn't run; gate 1 already passed, so the run goes on
    result.path.unlink()


@posix_only
def test_interpreter_gate_reports_an_empty_stderr_without_crashing(tmp_path, monkeypatch):
    class _Proc:
        returncode = 1
        stderr = b""

    monkeypatch.setattr(inject.subprocess, "run", lambda *a, **k: _Proc())
    with pytest.raises(InjectSyntaxError):
        inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert not temp_files(tmp_path)


# ---------------------------------------------------------------- $0 warning


@posix_only
def test_self_location_warns_when_a_temp_copy_is_written(tmp_path):
    src = '#!/usr/bin/env bash\nHERE=$(dirname "$0")\nWIDTH=800\necho "$HERE $WIDTH"\n'
    result = inject_src(src, {"WIDTH": "1200"}, tmp_path)
    assert result.path is not None
    assert len(result.warnings) == 1
    assert "$0" in result.warnings[0]


def test_self_location_does_not_warn_for_env_delivery(tmp_path):
    src = '#!/usr/bin/env bash\nHERE=$(dirname "$0")\necho "${MODE:-auto} $HERE"\n'
    result = inject_src(src, {"MODE": "manual"}, tmp_path)
    assert result.path is None  # no copy is written, so $0 is not affected at all
    assert result.warnings == []


def test_no_self_location_no_warning(tmp_path):
    result = inject_src("#!/usr/bin/env bash\nWIDTH=800\n", {"WIDTH": "1200"}, tmp_path)
    assert result.warnings == []
    assert result.path is not None
    result.path.unlink()


# ---------------------------------------------------------------- normalization


def test_normalize_rewrites_only_that_assignments_bytes():
    src = '#!/usr/bin/env bash\nWIDTH=800\nHEIGHT=600\necho "$WIDTH $HEIGHT"\n'
    result = normalize.normalize_idiom(src, ["WIDTH"])
    assert result.normalized == ["WIDTH"]
    assert result.refused == []
    assert (
        result.text
        == '#!/usr/bin/env bash\nWIDTH="${WIDTH:-800}"\nHEIGHT=600\necho "$WIDTH $HEIGHT"\n'
    )


def test_normalize_makes_the_param_an_envdefault():
    src = "#!/usr/bin/env bash\nWIDTH=800\n"
    out = normalize.normalize_idiom(src, ["WIDTH"]).text
    cands = {c.name: c for c in analyzer.analyze(out).candidates}
    assert cands["WIDTH"].binding == "envdefault"
    assert cands["WIDTH"].env_name == "WIDTH"
    assert cands["WIDTH"].default == 800  # the literal is still the script's standalone default


@posix_only
def test_normalized_script_still_runs_standalone(tmp_path):
    src = tmp_path / "s.sh"
    src.write_text(
        normalize.normalize_idiom(
            '#!/usr/bin/env bash\nGREETING=hello\necho "$GREETING"\n', ["GREETING"]
        ).text,
        encoding="utf-8",
    )
    assert run_shell(src, tmp_path).stdout == "hello\n"  # the default still applies...
    proc = subprocess.run(
        ["bash", str(src)],  # noqa: S607 — bash from PATH is exactly what the shell kind runs
        capture_output=True,
        text=True,
        env={"PATH": "/usr/bin:/bin", "GREETING": "hi"},
        check=False,
    )
    assert proc.stdout == "hi\n"  # ...and an inherited value now wins


@pytest.mark.parametrize(
    ("src", "code"),
    [
        ("A='literal $VAR'\n", "unsafe-literal"),  # a $ would start expanding once re-homed
        ("A='say \"hi\"'\n", "unsafe-literal"),  # a quote would close the wrapper's quote
        ("A='back\\slash'\n", "unsafe-literal"),
        ("A='tick `x`'\n", "unsafe-literal"),
        ("A='brace }'\n", "unsafe-literal"),  # would close the ${...} early
        ("readonly A=1\n", "readonly"),
        ("declare -r A=1\n", "readonly"),
        ("A=1\nA=2\n", "multiple-assignments"),
        ('A="${A:-1}"\n', "already-env"),  # it already IS the idiom
        ("B=1\n", "not-a-const"),
        ("A=$(date)\n", "not-a-const"),
        ('A="pre${OTHER}post"\n', "not-a-const"),  # no literal RHS at all
        ("A=\n", "not-a-const"),
        ("A+=1\n", "not-a-const"),
    ],
)
def test_normalize_refuses_and_leaves_the_source_untouched(src, code):
    result = normalize.normalize_idiom(src, ["A"])
    assert result.refused == [f"{code}:A"]
    assert result.normalized == []
    assert result.text == src  # byte-identical: a refusal never half-rewrites


def test_normalize_ignores_array_and_valueless_assignments():
    # Same agreement as the injector: a subscript target isn't a const, so it can't be normalized
    # (and `--normalize ARR` reports it rather than rewriting an array element).
    src = "#!/usr/bin/env bash\nARR[0]=1\nWIDTH=800\n"
    result = normalize.normalize_idiom(src, ["WIDTH", "ARR"])
    assert result.normalized == ["WIDTH"]
    assert result.refused == ["not-a-const:ARR"]
    assert result.text == '#!/usr/bin/env bash\nARR[0]=1\nWIDTH="${WIDTH:-800}"\n'


def test_normalize_on_an_unparseable_script_changes_nothing():
    src = "#!/usr/bin/env zsh\nif [[ -n $X ]] {\n  print hi\n}\nA=1\n"
    result = normalize.normalize_idiom(src, ["A"])
    assert result.refused == ["syntax-error:A"]
    assert result.text == src


def test_normalize_mixed_batch_reports_each_name():
    src = "#!/usr/bin/env bash\nWIDTH=800\nreadonly MAX=100\n"
    result = normalize.normalize_idiom(src, ["WIDTH", "MAX", "NOPE"])
    assert result.normalized == ["WIDTH"]
    assert result.refused == ["readonly:MAX", "not-a-const:NOPE"]
    assert 'WIDTH="${WIDTH:-800}"' in result.text
    assert "readonly MAX=100" in result.text


# ---------------------------------------------------------------- flows.execute integration


def _shell_entry(tmp_path: Path, text: str, *, name: str) -> store.Entry:
    src = tmp_path / f"{name}.sh"
    src.write_text(text, encoding="utf-8")
    return store.add_script(src, kind="shell", name=name)


@posix_only
def test_execute_runs_a_shell_entry_with_injected_values(tmp_path, capfd):
    _shell_entry(tmp_path, '#!/usr/bin/env bash\nWIDTH=800\necho "w=$WIDTH"\n', name="exsh1")
    assert runner.invoke(cli.app, ["params", "exsh1", "--manage", "WIDTH"]).exit_code == 0
    result = runner.invoke(cli.app, ["run", "exsh1", "--set", "WIDTH=1200", "--no-input"])
    assert result.exit_code == 0, result.output
    # The script's own stdout goes straight through the terminal (skit never wraps it), so it
    # lands on the real file descriptor — not in CliRunner's captured Rich output.
    assert "w=1200" in capfd.readouterr().out


@posix_only
def test_execute_runs_a_managed_read_with_the_block_in_place(tmp_path, capfd):
    """The whole loop, on a real store entry: `skit params --manage` writes the [tool.skit] block
    between the shebang and the code, and the preamble then has to land between the shebang and
    THAT — without breaking either. Nothing else proves the two writers compose."""
    _shell_entry(
        tmp_path,
        '#!/usr/bin/env bash\nread -s -p "Password: " PW\necho "len=${#PW}"\n',
        name="exsh1b",
    )
    assert runner.invoke(cli.app, ["params", "exsh1b", "--manage", "input-1"]).exit_code == 0
    stored = store.resolve("exsh1b").script_path.read_text(encoding="utf-8")
    assert "# /// script" in stored  # the block is there...
    result = runner.invoke(cli.app, ["run", "exsh1b", "--set", "input-1=hunter2", "--no-input"])
    assert result.exit_code == 0, result.output
    out = capfd.readouterr().out
    assert "Password: ***" in out  # ...and the secret read still injects, masked
    assert "len=7" in out
    assert "hunter2" not in out


@posix_only
def test_execute_env_delivery_writes_no_temp_copy(tmp_path, monkeypatch):
    from skit import launcher

    seen: dict[str, object] = {}

    def spy(entry, extra, *, values=None, invoke_cwd=None, script_override=None, env_overlay=None):
        seen["override"] = script_override
        seen["env"] = dict(env_overlay or {})
        return 0

    monkeypatch.setattr(launcher, "run_entry", spy)
    entry = _shell_entry(tmp_path, '#!/usr/bin/env bash\necho "${MODE:-auto}"\n', name="exsh2")
    assert runner.invoke(cli.app, ["params", "exsh2", "--manage", "MODE"]).exit_code == 0
    entry = store.resolve("exsh2")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {"MODE": "manual"}, [], cwd=tmp_path)
    outcome = flows.execute(entry, plan, asm, emit=lambda _line: None)
    assert outcome.code == 0
    assert seen["override"] is None  # no injected copy at all
    assert seen["env"] == {"MODE": "manual"}


@posix_only
def test_run_refuses_a_bad_value_before_it_ever_launches(tmp_path):
    _shell_entry(tmp_path, "#!/usr/bin/env bash\nWIDTH=800\n", name="exsh3")
    assert runner.invoke(cli.app, ["params", "exsh3", "--manage", "WIDTH"]).exit_code == 0
    bad = runner.invoke(cli.app, ["run", "exsh3", "--set", "WIDTH=abc", "--no-input"])
    assert bad.exit_code == flows.FAILURE_EXIT_CODES[flows.FAIL_BAD_VALUE]


@posix_only
def test_execute_maps_a_drifted_shell_definition_to_drift(tmp_path):
    """A definition whose const no longer exists: reconcile normally drops it from the form long
    before a run, so this drives execute() directly — the injector's last-resort refusal, and the
    one place the resync hint belongs."""
    entry = _shell_entry(tmp_path, "#!/usr/bin/env bash\nTALL=800\n", name="exsh3b")
    plan = flows.FormPlan(
        source="inject",
        fields=[flows.FormField(key="WIDTH", label="WIDTH")],
        specs=[ParamDecl(name="WIDTH", binding="const", delivery="inject", type="int")],
        text=entry.script_path.read_text(encoding="utf-8"),
    )
    outcome = flows.execute(
        entry,
        plan,
        flows.Assembly(inject_values={"WIDTH": "1200"}),
        emit=lambda _line: None,
    )
    assert outcome.failure == flows.FAIL_DRIFT
    assert "--resync" in outcome.message
    assert not temp_files(entry.dir)


@posix_only
def test_execute_reports_a_positional_gap_as_a_bad_value(tmp_path):
    _shell_entry(
        tmp_path,
        '#!/usr/bin/env bash\nread -p "First and last: " FIRST LAST\necho "$FIRST $LAST"\n',
        name="exsh4",
    )
    assert (
        runner.invoke(
            cli.app, ["params", "exsh4", "--manage", "input-1", "--manage", "input-2"]
        ).exit_code
        == 0
    )
    result = runner.invoke(cli.app, ["run", "exsh4", "--set", "input-2=Lovelace", "--no-input"])
    assert result.exit_code == flows.FAILURE_EXIT_CODES[flows.FAIL_BAD_VALUE]
    assert "input-1" in result.output


@posix_only
def test_execute_surfaces_the_self_location_warning(tmp_path):
    entry = _shell_entry(
        tmp_path,
        '#!/usr/bin/env bash\nHERE=$(dirname "$0")\nWIDTH=800\necho "$WIDTH"\n',
        name="exsh5",
    )
    assert runner.invoke(cli.app, ["params", "exsh5", "--manage", "WIDTH"]).exit_code == 0
    entry = store.resolve("exsh5")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {"WIDTH": "1200"}, [], cwd=tmp_path)
    lines: list[str] = []
    outcome = flows.execute(entry, plan, asm, emit=lines.append)
    assert outcome.code == 0
    assert any("$0" in line for line in lines)


@posix_only
def test_execute_syntax_gate_failure_never_launches(tmp_path, monkeypatch):
    from skit import launcher

    monkeypatch.setattr(inject, "quote", lambda value: f"'{value}")
    monkeypatch.setattr(
        launcher, "run_entry", lambda *a, **k: pytest.fail("the script must not launch")
    )
    entry = _shell_entry(tmp_path, "#!/usr/bin/env bash\nTITLE=hello\n", name="exsh6")
    assert runner.invoke(cli.app, ["params", "exsh6", "--manage", "TITLE"]).exit_code == 0
    entry = store.resolve("exsh6")
    plan = flows.plan_for_entry(entry)
    asm = flows.assemble(plan, {"TITLE": "x"}, [], cwd=tmp_path)
    outcome = flows.execute(entry, plan, asm, emit=lambda _line: None)
    assert outcome.failure == flows.FAIL_DRIFT
    assert "--resync" not in outcome.message  # a resync cannot fix skit's own corruption
    assert not temp_files(entry.dir)


def test_execute_without_an_injector_does_not_crash(tmp_path):
    """Defensive: an inject plan can only exist where an analyzer does, and every analyzer language
    ships an injector — but a kind that grew one without the other must degrade, not explode."""
    src = tmp_path / "s.fish"
    src.write_text("set -x NAME x\n", encoding="utf-8")
    entry = store.add_script(src, kind="fish", name="noinj")
    plan = flows.FormPlan(
        source="inject",
        fields=[flows.FormField(key="NAME", label="NAME")],
        specs=[ParamDecl(name="NAME", binding="const", delivery="inject")],
        text="set -x NAME x\n",
    )
    asm = flows.Assembly(inject_values={"NAME": "y"})
    lines: list[str] = []
    outcome = flows.execute(entry, plan, asm, emit=lines.append)
    # fish may not be installed; either it ran or the launch failed — never an injection crash.
    assert outcome.failure in (
        "",
        flows.FAIL_LAUNCH,
        flows.FAIL_MISSING,
        flows.FAIL_NOT_EXECUTABLE,
    )


# ---------------------------------------------------------------- CLI: --normalize


@posix_only
def test_cli_dry_run_shows_the_command(tmp_path):
    _shell_entry(tmp_path, '#!/usr/bin/env bash\nWIDTH=800\necho "$WIDTH"\n', name="cln1")
    assert runner.invoke(cli.app, ["params", "cln1", "--manage", "WIDTH"]).exit_code == 0
    result = runner.invoke(
        cli.app, ["run", "cln1", "--set", "WIDTH=1200", "--dry-run", "--no-input"]
    )
    assert result.exit_code == 0, result.output
    assert "WIDTH = 1200" in result.output
    assert "script.sh" in result.output  # the ORIGINAL path: a dry run writes no temp copy


def test_cli_normalize_turns_a_const_into_an_env_param(tmp_path):
    entry = _shell_entry(
        tmp_path, '#!/usr/bin/env bash\nWIDTH=800\nDEPTH=3\necho "$WIDTH $DEPTH"\n', name="cln2"
    )
    assert (
        runner.invoke(
            cli.app, ["params", "cln2", "--manage", "WIDTH", "--manage", "DEPTH"]
        ).exit_code
        == 0
    )
    result = runner.invoke(cli.app, ["params", "cln2", "--normalize", "WIDTH"])
    assert result.exit_code == 0, result.output
    text = entry.script_path.read_text(encoding="utf-8")
    assert 'WIDTH="${WIDTH:-800}"' in text
    assert "DEPTH=3" in text  # untouched
    assert 'kind = "envdefault"' in text  # the managed definition followed the source
    shown = runner.invoke(cli.app, ["show", "cln2", "--json"])
    assert shown.exit_code == 0, shown.output
    fields = {f["key"]: f for f in __import__("json").loads(shown.stdout)["fields"]}
    assert fields["WIDTH"]["source"] == "env"  # delivered by the environment from now on
    assert fields["DEPTH"]["source"] == "inject"


@posix_only
def test_cli_normalized_param_runs_through_the_environment(tmp_path, capfd):
    _shell_entry(tmp_path, '#!/usr/bin/env bash\nWIDTH=800\necho "w=$WIDTH"\n', name="cln3")
    assert runner.invoke(cli.app, ["params", "cln3", "--manage", "WIDTH"]).exit_code == 0
    assert runner.invoke(cli.app, ["params", "cln3", "--normalize", "WIDTH"]).exit_code == 0
    result = runner.invoke(cli.app, ["run", "cln3", "--set", "WIDTH=1200", "--no-input"])
    assert result.exit_code == 0, result.output
    assert "w=1200" in capfd.readouterr().out
    # The transparency line shows the honest picture: an env prefix, and the ORIGINAL script path.
    assert "WIDTH=1200 bash" in result.output
    assert ".injected-" not in result.output


def test_cli_normalize_reports_refusals(tmp_path):
    entry = _shell_entry(tmp_path, "#!/usr/bin/env bash\nreadonly MAX=100\n", name="cln4")
    before = entry.script_path.read_text(encoding="utf-8")
    result = runner.invoke(cli.app, ["params", "cln4", "--normalize", "MAX"])
    assert result.exit_code == 0
    assert "readonly" in result.output
    assert entry.script_path.read_text(encoding="utf-8") == before  # untouched


def test_cli_normalize_refuses_a_non_shell_kind(tmp_path):
    src = tmp_path / "s.py"
    src.write_text("WIDTH = 800\n", encoding="utf-8")
    store.add_python(src, name="cln5")
    result = runner.invoke(cli.app, ["params", "cln5", "--normalize", "WIDTH"])
    assert result.exit_code == 1
    assert "normalize" in result.output


def test_cli_normalize_refuses_reference_mode(tmp_path):
    src = tmp_path / "ref.sh"
    src.write_text("#!/usr/bin/env bash\nWIDTH=800\n", encoding="utf-8")
    store.add_script(src, kind="shell", name="cln6", mode="reference")
    result = runner.invoke(cli.app, ["params", "cln6", "--normalize", "WIDTH"])
    assert result.exit_code == 1
    assert "reference mode" in result.output
    assert src.read_text(encoding="utf-8") == "#!/usr/bin/env bash\nWIDTH=800\n"


def test_cli_normalize_without_a_stored_copy(tmp_path):
    entry = _shell_entry(tmp_path, "#!/usr/bin/env bash\nWIDTH=800\n", name="cln7")
    entry.script_path.unlink()
    result = runner.invoke(cli.app, ["params", "cln7", "--normalize", "WIDTH"])
    assert result.exit_code == 1
    assert "no stored copy" in result.output


def test_cli_normalize_warning_renderer_covers_every_code():
    for code in (
        "not-a-const",
        "multiple-assignments",
        "readonly",
        "already-env",
        "unsafe-literal",
        "syntax-error",
    ):
        assert cli._render_normalize_warning(f"{code}:X")


def test_split_guard_refuses_only_what_the_shell_would_actually_mangle(tmp_path):
    # The refusal set is measured against real shells, not Python's str.isspace():
    #   U+00A0 - whitespace to Python, but not a default-$IFS splitter: the shell keeps it whole.
    #   CR     - neither a splitter nor a line terminator: delivered byte-intact (verified with od).
    # Both must be ACCEPTED in a non-last field. Only space/tab (which split the line across the
    # fields) and newline (which ends the line) are refused there.
    src = '#!/usr/bin/env bash\nread -p "a b: " FIRST LAST\n'
    for accepted in ("a" + "\u00a0" + "b", "a\rb"):
        result = inject_src(src, {"input-1": accepted, "input-2": "x"}, tmp_path)
        assert result.path is not None  # accepted, not refused
    for splitter in (" ", "\t", "\n"):
        with pytest.raises(InjectSplitError):
            inject_src(src, {"input-1": f"a{splitter}b", "input-2": "x"}, tmp_path)


def test_params_warns_when_a_self_locating_script_has_injectable_consts(tmp_path):
    # $0/BASH_SOURCE: an injected const runs from a temp copy, so the script would see THAT path.
    # The user must learn this where they decide to manage the const — not only at run time — and
    # be pointed at --normalize (env delivery, file untouched).
    _shell_entry(
        tmp_path,
        '#!/usr/bin/env bash\nHERE=$(dirname "$0")\nREGION=us-east-1\necho "$HERE $REGION"\n',
        name="selfloc",
    )
    out = runner.invoke(cli.app, ["params", "selfloc"]).output.replace("\n", " ")
    assert "locates itself" in out
    assert "--normalize" in out


def test_params_does_not_warn_when_the_script_never_self_locates(tmp_path):
    _shell_entry(tmp_path, "#!/usr/bin/env bash\nREGION=us-east-1\necho $REGION\n", name="noloc")
    assert "locates itself" not in runner.invoke(cli.app, ["params", "noloc"]).output
