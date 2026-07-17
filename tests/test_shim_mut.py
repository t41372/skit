"""Mutation-kill tests for langs/python/shim.py — run-time parameter injection.

Every test drives the real `shim.inject` / `shim.inject_entry` surface and asserts an
observable result (injected text, delivered value, drift error, temp-file extension)."""

from __future__ import annotations

import subprocess
import sys

import pytest

from skit.langs.base import InjectRequest
from skit.langs.python import shim
from skit.params import Binding, ParamDecl, ParamType


def spec(
    name: str,
    *,
    binding: Binding = "const",
    type: ParamType = "str",
    order: int = -1,
    secret: bool = False,
    prompt: str = "",
) -> ParamDecl:
    return ParamDecl(
        name=name, binding=binding, type=type, order=order, secret=secret, prompt=prompt
    )


def _run_injected(source: str, stdin: str = "") -> str:
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


def test_stored_inputs_only_carries_inputs_that_have_a_value():
    # `stored_inputs` includes a spec only when it is an input binding AND has a value (`and`, not
    # `or`). input-1 (order 0) is value-less; input-2 (order 1) carries the value; both share the
    # prompt "Go? " and there is exactly ONE call site. Under the correct `and`, only input-2 is
    # stored, so it matches and binds the sole call and injects cleanly. Under `or`, the value-less
    # input-1 is also stored, exact-matches the only call first and claims it, and input-2 can no
    # longer bind — raising ShimError. A clean inject that delivers the value proves the filter.
    src = 'x = input("Go? ")\nprint(x)\n'
    specs = [
        spec("input-1", binding="input", order=0, prompt="Go? "),
        spec("input-2", binding="input", order=1, prompt="Go? "),
    ]
    out = shim.inject(src, specs, {"input-2": "hello"})
    assert 'x = _skit_i[0]("Go? ")' in out
    assert "hello" in _run_injected(out)


def test_a_drifted_input_does_not_abort_the_remaining_specs():
    # An input spec whose call site has drifted away (`binding is None`) is recorded missing, then
    # the loop must CONTINUE so later specs are still examined. A `break` there would swallow a
    # second, genuinely-missing const, hiding it from the drift report. Both names must surface.
    with pytest.raises(shim.ShimError) as exc:
        shim.inject(
            "print('hi')\n",
            [spec("DRIFT", binding="input", order=5), spec("GONE")],
            {"DRIFT": "x", "GONE": "y"},
        )
    message = str(exc.value)
    assert "DRIFT" in message
    assert "GONE" in message  # the trailing const is only reported if the loop did NOT break


def test_a_double_claimed_order_does_not_abort_the_remaining_specs():
    # Two input specs sharing order 0 both resolve to the one call site; the second finds its
    # resolved order already queued, is recorded missing, and the loop must CONTINUE so a later
    # drifted const is also reported. A `break` there would drop the trailing "GONE".
    src = 'x = input("Go? ")\nprint(x)\n'
    specs = [
        spec("input-1", binding="input", order=0, prompt="Go? "),
        spec("input-2", binding="input", order=0, prompt="Go? "),
        spec("GONE"),
    ]
    with pytest.raises(shim.ShimError) as exc:
        shim.inject(src, specs, {"input-1": "A", "input-2": "B", "GONE": "y"})
    assert "GONE" in str(exc.value)


def test_inject_entry_writes_a_dot_py_temp_copy(tmp_path):
    # inject_entry must pass write_injected suffix=".py" so `uv run --script` recognises the temp
    # copy by extension. Any other suffix ("XX.pyXX", ".PY", or None) breaks that contract.
    req = InjectRequest(
        text="CITY = 'x'\n", specs=[spec("CITY")], values={"CITY": "y"}, entry_dir=tmp_path
    )
    result = shim.inject_entry(req)
    try:
        assert result.path is not None
        assert result.path.name.startswith(".injected-")
        assert result.path.name.endswith(".py")  # exactly ".py", lower-case, present
        assert result.path.read_text(encoding="utf-8") == "CITY = 'y'\n"
    finally:
        if result.path is not None:
            result.path.unlink(missing_ok=True)
