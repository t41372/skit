"""Mutation-hardening tests for langs/javascript/inject.py.

Each test pins a concrete, observable behaviour of the real JS/TS injector (the pure
inject()/_gate_node()/_gate_reparse()/_injected_suffix()/_resolve_runner() surface) so that a
one-line corruption of the source is caught. They complement tests/test_js_inject.py; nothing here
mocks away the unit under test — the subprocess-boundary tests drive a real child process (a tiny
POSIX stand-in for `node`) or the analyzed injection on representative inputs. English catalog
(conftest pins SKIT_LANG=en) so message assertions are the source msgids verbatim.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from skit.langs.base import (
    InjectError,
    InjectRequest,
    InjectSyntaxError,
    InjectValueError,
)
from skit.langs.javascript import analyzer, inject
from skit.params import ParamDecl

posix_or_win = pytest.mark.skipif(sys.platform == "win32", reason="POSIX exec/mode assertion")


def specs_of(src: str, *, lang: str = "js") -> list[ParamDecl]:
    return [ParamDecl.from_candidate(c) for c in analyzer.analyze(src, lang=lang).candidates]


def inject_src(
    src: str,
    values: dict[str, str],
    tmp_path: Path,
    *,
    specs: list[ParamDecl] | None = None,
    lang: str = "js",
    interpreter: str = "",
    source: str = "",
) -> inject.InjectResult:
    return inject.inject(
        InjectRequest(
            text=src,
            specs=specs_of(src, lang=lang) if specs is None else specs,
            values=values,
            entry_dir=tmp_path,
            interpreter=interpreter,
            source=source,
        ),
        lang=lang,
    )


def temp_files(tmp_path: Path) -> list[Path]:
    return list(tmp_path.glob(".injected-*"))


def _fake_node(monkeypatch, returncode: int, stderr: bytes = b"") -> None:
    """Force gate 2 to resolve node and return a synthetic CompletedProcess (no real spawn)."""
    monkeypatch.setattr(inject, "_resolve_runner", lambda _i: ("node", "/fake/node"))

    class _Proc:
        def __init__(self) -> None:
            self.returncode = returncode
            self.stderr = stderr

    monkeypatch.setattr(inject.subprocess, "run", lambda *a, **k: _Proc())


def _fake_program(tmp_path: Path, body: str) -> Path:
    """A tiny executable POSIX stand-in for a runner, used to exercise the real subprocess call."""
    p = tmp_path / "fakerunner"
    p.write_text(f"#!/bin/sh\n{body}\n")
    p.chmod(0o755)
    return p


# ---------------------------------------------------------------- bad-value error payload


def test_bad_value_error_carries_the_raw_value_and_type(tmp_path):
    # InjectValueError must carry the offending value AND the declared type verbatim, so a caller
    # can word its own message without re-parsing str(exc). (Guards the raw/type_name arguments.)
    with pytest.raises(InjectValueError) as exc:
        inject_src("const W = 800;\n", {"W": "not-a-number"}, tmp_path)
    assert exc.value.value == "not-a-number"
    assert exc.value.type_name == "int"
    assert exc.value.param_name == "W"


# ---------------------------------------------------------------- const-target guard


def test_destructuring_binding_is_never_an_injection_target(tmp_path):
    # `const {a} = 5;` binds through an object_pattern, not a plain identifier — the value node (5)
    # IS a literal, so only the `name_node.type == "identifier"` conjunct keeps it from being
    # rewritten. Asking for a spec whose name equals the pattern text proves the guard bites: no
    # target is found, so it surfaces as drift (InjectError) and the RHS is left untouched.
    spec = ParamDecl(name="{a}", binding="const", delivery="inject", type="int")
    with pytest.raises(InjectError):
        inject_src("const {a} = 5;\n", {"{a}": "9"}, tmp_path, specs=[spec])
    assert not temp_files(tmp_path)


# ---------------------------------------------------------------- gate 2: real subprocess


@posix_or_win
def test_gate2_needs_captured_stderr_and_no_check_to_report_a_reject(tmp_path, monkeypatch):
    # A real runner that exits nonzero with stderr. capture_output=True makes proc.stderr readable
    # (without it stderr is None and the decode blows up); check=False lets us inspect the nonzero
    # returncode ourselves (check=True would raise CalledProcessError, swallowed as a gate that
    # "couldn't run"). Either regression turns the reported reject into a different outcome.
    fake = _fake_program(tmp_path, 'echo "SyntaxError: bad" >&2\nexit 1')
    monkeypatch.setattr(inject, "_resolve_runner", lambda _i: ("node", str(fake)))
    with pytest.raises(InjectSyntaxError) as exc:
        inject._gate_node("node", tmp_path / "x.js", ".js")
    assert "SyntaxError: bad" in str(exc.value)


def test_gate2_decodes_malformed_stderr_leniently(tmp_path, monkeypatch):
    # node's stderr need not be valid UTF-8. errors="replace" keeps the gate from crashing on
    # malformed bytes — strict decoding or a bogus handler name would raise instead of reporting.
    _fake_node(monkeypatch, 1, b"\xff\xfe not utf-8")
    with pytest.raises(InjectSyntaxError):
        inject._gate_node("node", tmp_path / "x.js", ".js")


def test_gate2_message_includes_the_first_stderr_line(tmp_path, monkeypatch):
    _fake_node(monkeypatch, 1, b"SyntaxError: boom\nmore\n")
    with pytest.raises(InjectSyntaxError) as exc:
        inject._gate_node("node", tmp_path / "x.js", ".js")
    assert str(exc.value) == "node rejected the injected copy: SyntaxError: boom"


def test_gate2_message_with_empty_stderr_has_a_blank_detail(tmp_path, monkeypatch):
    _fake_node(monkeypatch, 1, b"")
    with pytest.raises(InjectSyntaxError) as exc:
        inject._gate_node("node", tmp_path / "x.js", ".js")
    assert str(exc.value) == "node rejected the injected copy: "


def test_gate2_honors_the_pinned_interpreter(tmp_path, monkeypatch):
    # node pinned, and deno ALSO installed. Gate 2 must resolve the pinned node and run node --check.
    # If the pin were dropped (interpreter -> None at either the inject or _gate_node call), the
    # default deno>bun>node order would resolve deno first and skip the gate — leaving the copy
    # unchecked. We observe that the node program is the one actually spawned.
    monkeypatch.setattr(inject.shutil, "which", {"deno": "/x/deno", "node": "/x/node"}.get)
    spawned: list[str] = []

    class _Proc:
        returncode = 0
        stderr = b""

    monkeypatch.setattr(
        inject.subprocess, "run", lambda argv, *a, **k: (spawned.append(argv[0]), _Proc())[1]
    )
    result = inject_src("const W = 800;\n", {"W": "1200"}, tmp_path, interpreter="node")
    assert result.path is not None
    assert spawned == ["/x/node"]  # gate 2 ran node --check on the pinned runner
    result.path.unlink()


# ---------------------------------------------------------------- gate 1 message


def test_gate1_reparse_failure_message_is_the_exact_english_string():
    with pytest.raises(InjectSyntaxError) as exc:
        inject._gate_reparse("const X = ;\n", "js")  # a syntax error -> has_error
    assert str(exc.value) == (
        "the injected copy no longer parses as a JavaScript/TypeScript script (nothing was run)"
    )


# ---------------------------------------------------------------- suffix resolution


def test_injected_suffix_falls_back_to_js_for_an_unregistered_lang():
    # _SUFFIX.get(lang, ".js") keeps the resolver total: an unknown lang (no module flavor) yields
    # the plain ".js" default rather than None or a mangled extension.
    assert inject._injected_suffix("weird", "") == ".js"


def test_inject_defaults_lang_to_js(tmp_path, monkeypatch):
    # inject()'s lang defaults to "js"; a caller that omits it gets JS behaviour. Proven via a
    # .mjs-origin source: only lang="js" maps ("js","module") -> ".mjs"; a mutated default ("JS"/…)
    # misses _MODULE_SUFFIX and degrades the temp copy to a plain ".js".
    monkeypatch.setattr(inject, "_resolve_runner", lambda _i: (None, None))  # gate 2 skips
    req = InjectRequest(
        text="const N = 5;\n",
        specs=specs_of("const N = 5;\n"),
        values={"N": "7"},
        entry_dir=tmp_path,
        source="tool.mjs",
    )
    result = inject.inject(req)  # no lang -> default "js"
    assert result.path is not None
    assert result.path.suffix == ".mjs"
    result.path.unlink()


# ---------------------------------------------------------------- runner name normalization


def test_resolve_runner_strips_all_leading_path_segments(monkeypatch):
    # A pinned interpreter given as an absolute path normalizes to its bare basename: rsplit on the
    # LAST slash (split on the first would keep the leading directories).
    monkeypatch.setattr(inject.shutil, "which", lambda n: n)  # echo candidate as its resolved path
    name, program = inject._resolve_runner("/opt/tools/node")
    assert name == "node"
    assert program == "/opt/tools/node"


# ---------------------------------------------------------------- inject() control flow


def test_a_spec_without_a_value_does_not_stop_later_injection(tmp_path):
    # First loop: a spec with no supplied value is SKIPPED (continue), not a stop (break) — a later
    # spec that DOES have a value must still be injected.
    src = "const A = 1;\nconst B = 2;\n"  # analyze order is [A, B]
    result = inject_src(src, {"B": "9"}, tmp_path)  # A left alone, B supplied
    assert result.path is not None
    text = result.path.read_text(encoding="utf-8")
    assert "const B = 9;" in text
    assert "const A = 1;" in text  # untouched
    result.path.unlink()


def test_all_drifted_targets_are_collected_into_one_error(tmp_path):
    # Second loop: every missing target is collected (continue, not break) and joined with ", " into
    # a single drift error — both names must appear, exactly comma-space separated.
    specs = [
        ParamDecl(name="AA", binding="const", delivery="inject", type="str"),
        ParamDecl(name="BB", binding="const", delivery="inject", type="str"),
    ]
    with pytest.raises(InjectError) as exc:
        inject_src("const Z = 1;\n", {"AA": "x", "BB": "y"}, tmp_path, specs=specs)
    assert str(exc.value) == "AA, BB"


def test_a_refused_copy_is_removed_even_if_already_gone(tmp_path, monkeypatch):
    # The cleanup after a gate rejection must be missing-safe (unlink(missing_ok=True)): if the copy
    # is already gone, the ORIGINAL InjectSyntaxError must still surface — not a FileNotFoundError
    # masking it (which missing_ok=False/None would raise).
    def fake_gate(_interpreter, path, _suffix):
        path.unlink()  # the copy vanished before cleanup runs
        raise InjectSyntaxError("node said no")

    monkeypatch.setattr(inject, "_gate_node", fake_gate)
    with pytest.raises(InjectSyntaxError):
        inject_src("const W = 800;\n", {"W": "1200"}, tmp_path)
    assert not temp_files(tmp_path)
