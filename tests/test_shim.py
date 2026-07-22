"""Behavioural contract for shim injection: AST location, text substitution, all other bytes
unchanged."""

from __future__ import annotations

import pytest

from skit import rewrite
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
    out = shim.inject(SCRIPT, [spec("input-1", binding="input", order=0)], {"input-1": "Alice"})
    # 3a: the managed call site itself is rewritten (input(...) -> _skit_i[K](...)) and a
    # single-line preamble defines the one-shot per-call-site overrides.
    assert 'who = _skit_i[0]("Your name: ")' in out
    assert "# skit:shim" in out
    stdout = _run_injected(out)
    assert "Alice Taipei 3" in stdout
    assert "Your name: Alice" in stdout  # The prompt + injected value is echoed to mimic a terminal


def test_input_queue_preamble_is_single_line_after_docstring():
    out = shim.inject(SCRIPT, [spec("input-1", binding="input", order=0)], {"input-1": "Alice"})
    lines = out.splitlines()
    assert (
        lines[0] == '"""Docstring stays."""'
    )  # docstring is still line 0; __doc__ semantics preserved
    shim_lines = [ln for ln in lines if ln.endswith("# skit:shim")]
    assert len(shim_lines) == 1  # single physical line; line-number shift is always exactly 1
    assert '# dependencies = ["requests"]' in out  # PEP 723 block untouched


def test_input_queue_exhaustion_falls_back_to_stdin():
    src = "a = input('a: ')\nb = input('b: ')\nprint(a, b)\n"
    out = shim.inject(src, [spec("input-1", binding="input", order=0)], {"input-1": "one"})
    stdout = _run_injected(out, stdin="two\n")
    # Call 0 consumes the queue; call 1 falls back to native stdin pass-through.
    assert "one two" in stdout


def test_input_queue_in_loop_consumes_by_call_order():
    src = "vals = [input('v: ') for _ in range(3)]\nprint('|'.join(vals))\n"
    # The analyzer sees one input() call site (order 0), but it is invoked three times:
    # call 0 consumes the queue value; subsequent calls fall back to stdin. This is the key
    # advantage of the queue approach over in-place rewriting.
    out = shim.inject(src, [spec("input-1", binding="input", order=0)], {"input-1": "first"})
    stdout = _run_injected(out, stdin="second\nthird\n")
    assert "first|second|third" in stdout


def test_input_queue_secret_masks_echo():
    src = "token = input('token: ')\nprint('len', len(token))\n"
    out = shim.inject(
        src, [spec("input-1", binding="input", order=0, secret=True)], {"input-1": "hunter2"}
    )
    stdout = _run_injected(out)
    assert "hunter2" not in stdout  # Secret values must never be echoed
    assert "token: ***" in stdout
    assert "len 7" in stdout  # But the script itself receives the real value


def test_input_queue_with_future_import():
    src = '"""doc"""\nfrom __future__ import annotations\nx = input()\nprint(x)\n'
    out = shim.inject(src, [spec("input-1", binding="input", order=0)], {"input-1": "ok"})
    lines = out.splitlines()
    assert lines[1] == "from __future__ import annotations"  # preamble must go after __future__
    assert lines[2].endswith("# skit:shim")
    assert "ok" in _run_injected(out)


def test_input_queue_combined_with_const_injection():
    out = shim.inject(
        SCRIPT,
        [spec("CITY"), spec("input-1", binding="input", order=0)],
        {"CITY": "Tainan", "input-1": "Bob"},
    )
    assert "CITY = 'Tainan'" in out
    assert "Bob Tainan 3" in _run_injected(out)


def test_missing_value_leaves_script_untouched():
    out = shim.inject(SCRIPT, [spec("CITY")], {})
    assert out == SCRIPT


# ---------- shadowed `input`: the analyzer's guard, mirrored in the shim (A2) ----------


def test_shadowed_input_is_not_rewritten_and_surfaces_as_drift():
    # A script that binds `input` itself (a def) has NO managed call sites, so `_input_calls`
    # returns [] and a stored input spec can't resolve — it must surface as drift (ShimError)
    # rather than the shim splicing a stdin-fallback wrapper over the script's OWN function call.
    src = "def input(prompt=''):\n    return 'HARDCODED'\ny = input('Q: ')\nprint(y)\n"
    with pytest.raises(shim.ShimError):
        shim.inject(
            src, [spec("input-1", binding="input", order=0, prompt="Q: ")], {"input-1": "typed"}
        )


def test_shadowed_input_leaves_the_call_site_text_intact_when_only_a_const_is_delivered():
    # A const in the same shadowed-input file still injects, and the `input('Q: ')` call site is
    # left byte-for-byte intact (never rewritten to `_skit_i[K]`) because the shim treats the
    # bound name as the script's own function.
    src = (
        "def input(prompt=''):\n    return 'x'\nCITY = 'Taipei'\ny = input('Q: ')\nprint(y, CITY)\n"
    )
    out = shim.inject(src, [spec("CITY")], {"CITY": "Tainan"})
    assert "CITY = 'Tainan'" in out
    assert "y = input('Q: ')" in out  # untouched
    assert "_skit_i" not in out


def test_unshadowed_input_is_rewritten_to_the_wrapper():
    # Control: the SAME input spec against an unshadowed script DOES rewrite the call site, so the
    # shadow guard is not firing unconditionally / returning [] always.
    out = shim.inject(
        "y = input('Q: ')\nprint(y)\n",
        [spec("input-1", binding="input", order=0, prompt="Q: ")],
        {"input-1": "typed"},
    )
    assert "y = _skit_i[0]('Q: ')" in out
    assert "typed" in _run_injected(out)


def test_drifted_target_raises():
    with pytest.raises(shim.ShimError):
        shim.inject(SCRIPT, [spec("GONE")], {"GONE": "x"})


def test_bad_type_coercion_raises():
    with pytest.raises(shim.ShimError):
        shim.inject(SCRIPT, [spec("RETRIES", type="int")], {"RETRIES": "not-a-number"})


def test_bad_type_coercion_raises_the_value_subclass_not_plain_shim_error():
    # A bad value is a distinct failure mode from a missing/drifted target: the target (RETRIES)
    # WAS found; only the supplied value doesn't fit its declared int type. Callers (the CLI) need
    # to tell the two apart to avoid misdiagnosing a bad input as source drift, so this must raise
    # the ShimValueError subclass specifically, carrying the value/type/param for the caller's
    # message -- not just the generic base ShimError raised for a genuinely missing target.
    with pytest.raises(shim.ShimValueError) as exc_info:
        shim.inject(SCRIPT, [spec("RETRIES", type="int")], {"RETRIES": "not-a-number"})
    exc = exc_info.value
    assert isinstance(exc, shim.ShimError)  # still a ShimError: existing `except ShimError` holds
    assert exc.value == "not-a-number"
    assert exc.type_name == "int"
    assert exc.param_name == "RETRIES"


def test_drifted_target_raises_plain_shim_error_not_value_subclass():
    # The converse: a genuinely missing target must NOT be reported as ShimValueError (that would
    # wrongly suggest to a caller that the value, not the target, was the problem).
    with pytest.raises(shim.ShimError) as exc_info:
        shim.inject(SCRIPT, [spec("GONE")], {"GONE": "x"})
    assert not isinstance(exc_info.value, shim.ShimValueError)


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
        shim.inject(src, [spec("input-1", binding="input", order=5)], {"input-1": "x"})


# ---------- inject: duplicate-prompt specs must never double-bind onto one call site (regression) ----------


def test_inject_two_identical_prompts_one_deleted_raises_cleanly_never_corrupts():
    # Regression: input-1 and input-2 both stored prompt "Go? "; the first of the two input()
    # calls was deleted, leaving one current call site with that prompt. Pre-fix, both specs
    # exact-matched onto that single call site and inject() spliced two replacements over the
    # same `input` callee span, producing corrupt source like `x = _skit_i[0]_i[0]("Go? ")` that
    # fails compile(). The surplus spec must now be reported as drift (ShimError), and the output
    # must never contain a doubled callee.
    src = 'first = input("Go? ")\nsecond = input("Go? ")\nprint(first, second)\n'
    edited = src.replace('first = input("Go? ")\n', "")  # delete the first call
    specs = [
        spec("input-1", binding="input", order=0, prompt="Go? "),
        spec("input-2", binding="input", order=1, prompt="Go? "),
    ]
    with pytest.raises(shim.ShimError):
        shim.inject(edited, specs, {"input-1": "AAA", "input-2": "BBB"})


def test_inject_duplicate_prompt_winner_only_still_injects_and_compiles():
    # The healthy end of the same scenario: once reconcile has dropped the surplus spec (as it now
    # correctly does, see test_reconcile), injecting only the surviving spec must still work
    # normally and produce compilable output with a single, non-doubled callee.
    src = 'first = input("Go? ")\nsecond = input("Go? ")\nprint(first, second)\n'
    edited = src.replace('first = input("Go? ")\n', "")
    out = shim.inject(
        edited, [spec("input-1", binding="input", order=0, prompt="Go? ")], {"input-1": "AAA"}
    )
    assert "_skit_i[0]_i[0]" not in out
    compile(out, "<test>", "exec")


def test_inject_specs_sharing_the_same_order_never_double_bind():
    # Defense-in-depth at the shim layer itself: two ParamDecl entries that carry the identical
    # `order` (e.g. a hand-edited or otherwise corrupted [tool.skit] block) look up the exact same
    # match_calls binding and would both try to queue a replacement over the same input() callee
    # span. inject() must refuse to emit the second, overlapping replacement -- reporting drift via
    # ShimError instead of corrupting the temp copy.
    src = 'x = input("Go? ")\nprint(x)\n'
    specs = [
        spec("input-1", binding="input", order=0, prompt="Go? "),
        spec("input-2", binding="input", order=0, prompt="Go? "),
    ]
    with pytest.raises(shim.ShimError):
        shim.inject(src, specs, {"input-1": "AAA", "input-2": "BBB"})


def test_inject_triple_duplicate_specs_same_order_never_double_bind():
    src = 'x = input("Go? ")\nprint(x)\n'
    specs = [
        spec("input-1", binding="input", order=0, prompt="Go? "),
        spec("input-2", binding="input", order=0, prompt="Go? "),
        spec("input-3", binding="input", order=0, prompt="Go? "),
    ]
    with pytest.raises(shim.ShimError):
        shim.inject(src, specs, {"input-1": "AAA", "input-2": "BBB", "input-3": "CCC"})


# ---------- _insert_preamble: empty body inserts at end ----------


def test_preamble_inserted_at_end_for_no_docstring_no_future():
    """When the source has no docstring and no __future__ import, the preamble goes right before
    the first real statement (which is at index 0, so the preamble is inserted at the top)."""
    # A file with only a bare input() call has no docstring and no __future__ import,
    # so _preamble_line_index returns 0: the preamble is inserted before line 0.
    src = "x = input('v: ')\nprint(x)\n"
    out = shim.inject(src, [spec("input-1", binding="input", order=0)], {"input-1": "hi"})
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


# ---------- _physical_lines: AST-line-boundary characters str.splitlines() over-splits on ----------


def test_physical_lines_matches_splitlines_on_ordinary_text():
    """On text with only real newlines, _physical_lines must agree with str.splitlines(keepends=
    True) exactly (empty input, no trailing newline, and a trailing newline all round-trip)."""
    for text in ("", "a", "a\nb", "a\nb\n", "a\r\nb\rc\n"):
        assert rewrite._physical_lines(text) == text.splitlines(keepends=True)


def test_const_injection_survives_form_feed_between_targets():
    """A form-feed page break (e.g. an Emacs section marker) sits on its own physical line as far
    as str.splitlines() is concerned, but the tokenizer/AST do NOT count it as a line break — so
    indexing lines[lineno - 1] from splitlines() output lands on the wrong physical line entirely.
    Reproduces the corruption: PORT's replacement used to land one physical line early (the form
    feed's own splitlines() "line"), producing `\\x0c\\n9090PORT = 8080` and a SyntaxError."""
    src = 'HOST = "localhost"\n\x0c\nPORT = 8080\nprint(HOST, PORT)\n'
    out = shim.inject(src, [spec("PORT", type="int")], {"PORT": "9090"})
    assert out == 'HOST = "localhost"\n\x0c\nPORT = 9090\nprint(HOST, PORT)\n'
    compile(out, "<test>", "exec")  # used to raise SyntaxError before the fix


def test_const_injection_survives_u2028_inside_earlier_string_literal():
    """U+2028 (LINE SEPARATOR) is an ordinary character inside a Python string literal -- it does
    not end the string, and the tokenizer does not treat it as a line break. str.splitlines(),
    however, always treats it as one, so a line count computed that way silently disagrees with the
    AST from the very next statement onward."""
    src = 'MSG = "hi\u2028there"\nPORT = 8080\nprint(MSG, PORT)\n'
    out = shim.inject(src, [spec("PORT", type="int")], {"PORT": "9090"})
    assert out == 'MSG = "hi\u2028there"\nPORT = 9090\nprint(MSG, PORT)\n'
    compile(out, "<test>", "exec")


def test_preamble_insertion_survives_form_feed_inside_docstring():
    """A form feed embedded inside the module docstring makes str.splitlines() split the
    docstring into two entries, so the *_insert_preamble*
    index (computed from the true, 1-entry-per-docstring AST line count) lands one entry early --
    squarely inside the docstring's text. The result still compiles (it's still valid Python), but
    input() is never actually overridden, and the queued value is silently dropped with no error at
    all -- the worst kind of failure this fix exists to prevent."""
    src = '"""line one\x0cline two"""\nname = input("who: ")\nprint(name)\n'
    out = shim.inject(src, [spec("input-1", binding="input", order=0)], {"input-1": "Bob"})
    # The docstring must be left intact as a single statement (the preamble must NOT have landed
    # inside it): it must still be the true first line, unsplit, before any skit-injected text.
    assert out.startswith('"""line one\x0cline two"""\n')
    assert "# skit:shim" in out
    compile(out, "<test>", "exec")
    assert "Bob" in _run_injected(out)  # the value actually reaches the script, not stdin


# ---------- Input values are bound to their prompt/call site, not runtime call order ----------


def test_input_value_follows_prompt_despite_runtime_call_order_diverging_from_source_order():
    """A function's input() is defined ABOVE a top-level input() in source order, but only
    invoked (at runtime) AFTER it runs. The old design
    queued/consumed values by a single global runtime counter keyed to *source* order, so the
    top-level call (which actually runs first) stole the function's queued value and vice versa --
    silently swapping a secret into the wrong variable. Binding by call site (not a shared counter)
    must keep each value with its own question regardless of execution order."""
    src = (
        "def get_password():\n"
        '    return input("Password: ")\n'
        "\n"
        'username = input("Username: ")\n'
        "password = get_password()\n"
        "print(username, password)\n"
    )
    specs = [
        spec("input-1", binding="input", order=0, secret=True),  # "Password: ", defined first
        spec("input-2", binding="input", order=1),  # "Username: ", defined second, RUNS first
    ]
    out = shim.inject(src, specs, {"input-1": "SUPERSECRET", "input-2": "alice"})
    stdout = _run_injected(out)
    assert "alice SUPERSECRET" in stdout  # username=alice, password=SUPERSECRET — not swapped
    assert "SUPERSECRET" not in stdout.replace("alice SUPERSECRET", "")  # only echoed as ***
    assert "Password: ***" in stdout
    assert "Username: alice" in stdout


def test_input_value_follows_prompt_after_an_earlier_input_is_deleted():
    """Reproduces the reconcile/shim "positional key is unstable under source edits" gap: a stored
    definition for the SECOND question ("Password: ", order=1, secret) must keep landing on the
    Password prompt even after the user deletes the FIRST input() call from the source, which shifts
    every remaining call's bare position down by one. Prompt-based matching (3a) resolves this
    without needing the caller to re-add anything."""
    original_order = 1  # as recorded when both input() calls existed
    edited_src = 'password = input("Password: ")\nprint("got", password)\n'  # first input() deleted
    stored_specs = [
        spec("input-2", binding="input", order=original_order, secret=True, prompt="Password: ")
    ]
    out = shim.inject(edited_src, stored_specs, {"input-2": "hunter2"})
    stdout = _run_injected(out)
    assert "got hunter2" in stdout
    assert "Password: ***" in stdout  # still masked as a secret, still the right prompt


# ---------- write_injected: exception during write cleans up the temp file ----------


def test_write_injected_cleanup_on_error(tmp_path, monkeypatch):
    """If the write fails, the temp file must be deleted (no orphan files)."""
    import os
    import tempfile

    # 3b moved the primary write target to the OS temp directory; redirect it to tmp_path so the
    # glob below still observes exactly what write_injected did (hermetic, no real /tmp writes).
    monkeypatch.setattr(tempfile, "tempdir", str(tmp_path))

    def bad_fdopen(fd, *a, **kw):
        os.close(fd)
        raise OSError("disk full")

    monkeypatch.setattr(os, "fdopen", bad_fdopen)
    with pytest.raises(OSError, match="disk full"):
        rewrite.write_injected(tmp_path, "print(1)\n", suffix=".py")
    # No .injected-*.py file should remain
    leftovers = list(tmp_path.glob(".injected-*.py"))
    assert leftovers == []


# ---------- write_injected: fd leak when os.chmod raises before fdopen (nit fix) ----------


def test_write_injected_closes_fd_when_chmod_raises(tmp_path, monkeypatch):
    """If os.chmod raises before the fd is handed to fdopen, the fd must still be closed --
    otherwise it leaks for the life of the process (repeated runs would accumulate leaked fds)."""
    import os
    import tempfile

    monkeypatch.setattr(tempfile, "tempdir", str(tmp_path))
    captured: dict[str, int] = {}
    real_mkstemp = tempfile.mkstemp

    def spy_mkstemp(*a, **kw):
        fd, path = real_mkstemp(*a, **kw)
        captured["fd"] = fd
        return fd, path

    monkeypatch.setattr(tempfile, "mkstemp", spy_mkstemp)

    def bad_chmod(path, mode):
        raise OSError("permission denied")

    monkeypatch.setattr(os, "chmod", bad_chmod)

    with pytest.raises(OSError, match="permission denied"):
        rewrite.write_injected(tmp_path, "print(1)\n", suffix=".py")

    # If write_injected leaked the fd, this close would succeed; a real close-on-error means the
    # fd is already invalid, so closing it again must fail with EBADF.
    with pytest.raises(OSError, match="Bad file descriptor"):
        os.close(captured["fd"])
    assert list(tmp_path.glob(".injected-*.py")) == []


# ---------- write_injected: 3b — the temp file no longer lives in the persistent store ----------


def test_write_injected_lands_outside_entry_dir(tmp_path):
    """The injected copy (which may contain plaintext secret values) must land in the OS temp
    directory, not next to the persistent script store: a SIGKILL/OOM/power-loss before the
    caller's `finally: unlink()` runs must never leave a plaintext-secret file sitting forever in
    entry_dir, since nothing skit owns ever sweeps it (unlike the OS's own temp directory)."""
    path = rewrite.write_injected(tmp_path, "print(1)\n", suffix=".py")
    try:
        assert path.parent != tmp_path
        assert not (tmp_path / path.name).exists()
        assert path.name.startswith(".injected-")
        assert path.read_text(encoding="utf-8") == "print(1)\n"
    finally:
        path.unlink(missing_ok=True)


def test_write_injected_falls_back_to_entry_dir_if_os_temp_unavailable(tmp_path, monkeypatch):
    """entry_dir is kept as a defense-in-depth fallback: if the OS temp directory can't be used
    (e.g. TMPDIR misconfigured), the run must still succeed rather than fail outright."""
    import tempfile

    real_mkstemp = tempfile.mkstemp

    def flaky_mkstemp(*args, **kwargs):
        if kwargs.get("dir") is None:  # the primary (OS-temp) attempt
            raise OSError("no system temp directory available")
        return real_mkstemp(*args, **kwargs)

    monkeypatch.setattr(tempfile, "mkstemp", flaky_mkstemp)
    path = rewrite.write_injected(tmp_path, "print(1)\n", suffix=".py")
    try:
        assert path.parent == tmp_path
    finally:
        path.unlink(missing_ok=True)
