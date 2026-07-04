"""Behavioural contract for shim injection: AST location, text substitution, all other bytes
unchanged."""

from __future__ import annotations

import pytest

from skit import shim
from skit.metawriter import ParamSpec


def spec(
    name: str, *, kind: str = "const", type: str = "str", order: int = -1, secret: bool = False
) -> ParamSpec:
    return ParamSpec(name=name, kind=kind, type=type, order=order, secret=secret)


SCRIPT = '''"""Docstring stays."""
# /// script
# dependencies = ["requests"]
# ///
CITY = "Taipei"  # trailing comment stays
RETRIES = 3

def main():
    who = input("Your name: ")
    print(who, CITY, RETRIES)

if __name__ == "__main__":
    DEBUG = True
    main()
'''


def test_const_str_injection_preserves_everything_else():
    out = shim.inject(SCRIPT, [spec("CITY")], {"CITY": "Kaohsiung"})
    assert "CITY = 'Kaohsiung'  # trailing comment stays" in out
    assert '# dependencies = ["requests"]' in out
    assert "RETRIES = 3" in out


def test_const_typed_injection():
    out = shim.inject(SCRIPT, [spec("RETRIES", type="int")], {"RETRIES": "7"})
    assert "RETRIES = 7" in out


def test_main_guard_const():
    out = shim.inject(SCRIPT, [spec("DEBUG", type="bool")], {"DEBUG": "false"})
    assert "DEBUG = False" in out


def _run_injected(source: str, stdin: str = "") -> str:
    """Run the injected output in a subprocess and return stdout (behaviour verification)."""
    import subprocess
    import sys

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


def test_input_queue_by_order():
    out = shim.inject(SCRIPT, [spec("input-1", kind="input", order=0)], {"input-1": "Alice"})
    # Phase 3: input() calls are not rewritten in-place; instead a single-line intercept queue
    # preamble is inserted.
    assert "input(" in out
    assert "# skit:shim" in out
    stdout = _run_injected(out)
    assert "Alice Taipei 3" in stdout
    assert "Your name: Alice" in stdout  # The prompt + injected value is echoed to mimic a terminal


def test_input_queue_preamble_is_single_line_after_docstring():
    out = shim.inject(SCRIPT, [spec("input-1", kind="input", order=0)], {"input-1": "Alice"})
    lines = out.splitlines()
    assert (
        lines[0] == '"""Docstring stays."""'
    )  # docstring is still line 0; __doc__ semantics preserved
    shim_lines = [ln for ln in lines if ln.endswith("# skit:shim")]
    assert len(shim_lines) == 1  # single physical line; line-number shift is always exactly 1
    assert '# dependencies = ["requests"]' in out  # PEP 723 block untouched


def test_input_queue_exhaustion_falls_back_to_stdin():
    src = "a = input('a: ')\nb = input('b: ')\nprint(a, b)\n"
    out = shim.inject(src, [spec("input-1", kind="input", order=0)], {"input-1": "one"})
    stdout = _run_injected(out, stdin="two\n")
    # Call 0 consumes the queue; call 1 falls back to native stdin pass-through.
    assert "one two" in stdout


def test_input_queue_in_loop_consumes_by_call_order():
    src = "vals = [input('v: ') for _ in range(3)]\nprint('|'.join(vals))\n"
    # The analyzer sees one input() call site (order 0), but it is invoked three times:
    # call 0 consumes the queue value; subsequent calls fall back to stdin. This is the key
    # advantage of the queue approach over in-place rewriting.
    out = shim.inject(src, [spec("input-1", kind="input", order=0)], {"input-1": "first"})
    stdout = _run_injected(out, stdin="second\nthird\n")
    assert "first|second|third" in stdout


def test_input_queue_secret_masks_echo():
    src = "token = input('token: ')\nprint('len', len(token))\n"
    out = shim.inject(
        src, [spec("input-1", kind="input", order=0, secret=True)], {"input-1": "hunter2"}
    )
    stdout = _run_injected(out)
    assert "hunter2" not in stdout  # Secret values must never be echoed
    assert "token: ***" in stdout
    assert "len 7" in stdout  # But the script itself receives the real value


def test_input_queue_with_future_import():
    src = '"""doc"""\nfrom __future__ import annotations\nx = input()\nprint(x)\n'
    out = shim.inject(src, [spec("input-1", kind="input", order=0)], {"input-1": "ok"})
    lines = out.splitlines()
    assert lines[1] == "from __future__ import annotations"  # preamble must go after __future__
    assert lines[2].endswith("# skit:shim")
    assert "ok" in _run_injected(out)


def test_input_queue_combined_with_const_injection():
    out = shim.inject(
        SCRIPT,
        [spec("CITY"), spec("input-1", kind="input", order=0)],
        {"CITY": "Tainan", "input-1": "Bob"},
    )
    assert "CITY = 'Tainan'" in out
    assert "Bob Tainan 3" in _run_injected(out)


def test_missing_value_leaves_script_untouched():
    out = shim.inject(SCRIPT, [spec("CITY")], {})
    assert out == SCRIPT


def test_drifted_target_raises():
    with pytest.raises(shim.ShimError):
        shim.inject(SCRIPT, [spec("GONE")], {"GONE": "x"})


def test_bad_type_coercion_raises():
    with pytest.raises(shim.ShimError):
        shim.inject(SCRIPT, [spec("RETRIES", type="int")], {"RETRIES": "not-a-number"})


def test_multiline_value_span():
    # Parenthesised literal: the AST span covers the literal only; the parens are preserved
    # (semantically equivalent).
    src = 'MSG = (\n    "hello"\n)\nprint(MSG)\n'
    out = shim.inject(src, [spec("MSG")], {"MSG": "bye"})
    assert "'bye'" in out
    assert '"hello"' not in out
    compile(out, "<test>", "exec")  # Injected output must still be valid Python


# ---------- _coerce_bool: invalid string ----------


def test_coerce_bool_invalid_raises():
    """Any string not in the recognized set must raise ShimError, not return a falsy value."""
    with pytest.raises(shim.ShimError):
        shim.inject("FLAG = True\n", [spec("FLAG", type="bool")], {"FLAG": "maybe"})


# ---------- inject: SyntaxError in source raises ShimError ----------


def test_inject_syntax_error_raises():
    with pytest.raises(shim.ShimError):
        shim.inject("def broken(:\n", [spec("X")], {"X": "1"})


# ---------- inject: input order beyond available calls is drift ----------


def test_input_order_beyond_calls_is_drift():
    """order=5 when there are no input() calls means the definition drifted."""
    src = "print('hello')\n"
    with pytest.raises(shim.ShimError):
        shim.inject(src, [spec("input-1", kind="input", order=5)], {"input-1": "x"})


# ---------- _insert_preamble: empty body inserts at end ----------


def test_preamble_inserted_at_end_for_no_docstring_no_future():
    """When the source has no docstring and no __future__ import, the preamble goes right before
    the first real statement (which is at index 0, so the preamble is inserted at the top)."""
    # A file with only a bare input() call has no docstring and no __future__ import,
    # so _preamble_line_index returns 0: the preamble is inserted before line 0.
    src = "x = input('v: ')\nprint(x)\n"
    out = shim.inject(src, [spec("input-1", kind="input", order=0)], {"input-1": "hi"})
    assert "# skit:shim" in out
    lines = out.splitlines()
    # The preamble must be the very first line (index 0)
    assert lines[0].endswith("# skit:shim")


# ---------- _apply: multi-line (cross-line) span replacement ----------


def test_multiline_span_replacement():
    """Parenthesised literal spanning two lines must be replaced cleanly."""
    src = 'X = (\n    "old"\n    "also old"\n)\nprint(X)\n'
    out = shim.inject(src, [spec("X")], {"X": "new"})
    assert "'new'" in out
    compile(out, "<test>", "exec")


# ---------- write_injected: exception during write cleans up the temp file ----------


def test_write_injected_cleanup_on_error(tmp_path, monkeypatch):
    """If the write fails, the temp file must be deleted (no orphan files)."""
    import os

    def bad_fdopen(fd, *a, **kw):
        os.close(fd)
        raise OSError("disk full")

    monkeypatch.setattr(os, "fdopen", bad_fdopen)
    with pytest.raises(OSError, match="disk full"):
        shim.write_injected(tmp_path, "print(1)\n")
    # No .injected-*.py file should remain
    leftovers = list(tmp_path.glob(".injected-*.py"))
    assert leftovers == []
