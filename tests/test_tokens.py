"""Value-token engine: expansion, escapes, pass-through, and error contracts."""

from __future__ import annotations

from datetime import datetime
from pathlib import Path

import pytest

from skit import tokens

NOW = datetime(2026, 7, 9, 14, 30, 5)
CWD = Path("/work/dir")


def _expand(text: str, env: dict[str, str] | None = None) -> str:
    return tokens.expand(text, cwd=CWD, env=env or {}, now=NOW)


def test_cwd_token():
    # {cwd} expands to str(cwd), which is platform-native (backslashes on Windows) — so pin the
    # expectation to str(CWD), not a hardcoded POSIX prefix.
    assert _expand("{cwd}/out.png") == f"{CWD}/out.png"


def test_today_token():
    assert _expand("report_{today}.csv") == "report_2026-07-09.csv"


def test_now_token():
    assert _expand("run_{now}.log") == "run_14-30-05.log"


def test_env_token_present():
    assert _expand("key={env:API_KEY}", {"API_KEY": "abc123"}) == "key=abc123"


def test_env_token_missing_raises_with_names():
    with pytest.raises(tokens.TokenError) as exc:
        _expand("{env:MISSING_VAR}")
    # Pin the whole sentence, not just the substrings: both interpolated fields (the bare name
    # and the full token) *and* the exact prose/casing, so a corrupted or down-cased message
    # string can't slip through.
    assert str(exc.value) == (
        "The environment variable MISSING_VAR isn't set (needed by {env:MISSING_VAR})."
    )


def test_multiple_tokens_in_one_value():
    assert _expand("{cwd}/out_{today}_{now}.png") == f"{CWD}/out_2026-07-09_14-30-05.png"


def test_unknown_braces_pass_through():
    # A value may carry braces meant for the script itself; only known tokens expand.
    assert _expand("pattern_{frame}.png") == "pattern_{frame}.png"
    assert _expand("{envelope}") == "{envelope}"


def test_double_brace_escapes():
    assert _expand("{{cwd}}") == "{cwd}"
    assert _expand("a{{b}}c") == "a{b}c"


def test_brace_escapes_false_keeps_double_braces_byte_identical():
    # Placeholder-delivery mode: `{{`/`}}` pass through untouched (prompt text is
    # brace-heavy; unmanaged text travels byte-identical).
    assert tokens.expand("{{cwd}}", cwd=CWD, env={}, now=NOW, brace_escapes=False) == "{{cwd}}"
    assert tokens.expand("a{{b}}c", cwd=CWD, env={}, now=NOW, brace_escapes=False) == "a{{b}}c"


def test_brace_escapes_true_halves_the_pair():
    assert tokens.expand("{{cwd}}", cwd=CWD, env={}, now=NOW, brace_escapes=True) == "{cwd}"


def test_named_tokens_expand_in_both_brace_modes():
    # The escape-pair policy is orthogonal to the named tokens: {cwd} expands regardless.
    assert tokens.expand("{cwd}/x", cwd=CWD, env={}, now=NOW, brace_escapes=False) == f"{CWD}/x"
    assert tokens.expand("{cwd}/x", cwd=CWD, env={}, now=NOW, brace_escapes=True) == f"{CWD}/x"


def test_preview_threads_brace_escapes():
    # The preview must take the SAME brace_escapes the delivery will, or it shows a lie.
    assert tokens.preview("{{cwd}}", cwd=CWD, env={}, now=NOW, brace_escapes=False) == (
        "{{cwd}}",
        None,
    )
    assert tokens.preview("{{cwd}}", cwd=CWD, env={}, now=NOW, brace_escapes=True) == (
        "{cwd}",
        None,
    )


def test_tilde_expansion_only_at_start(monkeypatch):
    monkeypatch.setenv("HOME", "/home/u")  # POSIX home
    monkeypatch.setenv("USERPROFILE", "/home/u")  # Windows home (os.path.expanduser reads this)
    assert _expand("~/x.txt") == "/home/u/x.txt"
    assert _expand("~") == "/home/u"
    assert _expand("a~b") == "a~b"  # not a home reference; untouched


def test_tilde_then_tokens_compose(monkeypatch):
    monkeypatch.setenv("HOME", "/home/u")
    monkeypatch.setenv("USERPROFILE", "/home/u")
    assert _expand("~/out_{today}.png") == "/home/u/out_2026-07-09.png"


def test_plain_text_unchanged():
    assert _expand("just a value, nothing special") == "just a value, nothing special"


def test_preview_success_and_failure():
    ok, err = tokens.preview("x_{today}", cwd=CWD, env={}, now=NOW)
    assert (ok, err) == ("x_2026-07-09", None)
    orig, err = tokens.preview("{env:NOPE}", cwd=CWD, env={}, now=NOW)
    assert orig == "{env:NOPE}"
    assert err is not None
    assert "NOPE" in err


def test_has_tokens():
    assert tokens.has_tokens("{cwd}/x")
    assert tokens.has_tokens("~/x")
    assert tokens.has_tokens("a{{b")
    assert tokens.has_tokens("a}}b")  # a closing-brace escape alone still means expand() acts
    assert tokens.has_tokens("{env:A}")
    assert not tokens.has_tokens("plain")
    assert not tokens.has_tokens("pattern_{frame}.png")  # unknown token: expand() is a no-op


def test_default_env_and_now_paths(monkeypatch):
    # Defaults resolve from the real environment/clock; pin just the env var.
    monkeypatch.setenv("SKIT_TOKEN_TEST", "v")
    assert tokens.expand("{env:SKIT_TOKEN_TEST}", cwd=CWD) == "v"
    out = tokens.expand("{today}", cwd=CWD)
    assert len(out) == 10
    assert out[4] == "-"
    assert out[7] == "-"


# --------------------------------------------------------------------------
# mutation hardening
# --------------------------------------------------------------------------


def test_escape_sequences_mid_string_exact():
    assert _expand("x{{y}}z") == "x{y}z"
    assert _expand("{{{{") == "{{"  # two escapes back to back
    assert _expand("}}{{") == "}{"
    assert _expand("a{{") == "a{"  # trailing opener escape
    assert _expand("{today}}}") == "2026-07-09}"  # token then escape


def test_preview_forwards_every_argument():
    # cwd forwarded
    assert tokens.preview("{cwd}", cwd=CWD, env={}, now=NOW) == (str(CWD), None)
    # env forwarded (a dropped env kwarg would fall back to os.environ and miss K)
    assert tokens.preview("{env:K}", cwd=CWD, env={"K": "v"}, now=NOW) == ("v", None)
    # now forwarded
    assert tokens.preview("{now}", cwd=CWD, env={}, now=NOW) == ("14-30-05", None)


def test_escape_deep_in_string_exact():
    # Escapes far from index 0/2 pin the scanner's advance arithmetic: a mutant that
    # rewinds or pins the index re-reads earlier characters and corrupts the output.
    assert _expand("abc{{d}}e") == "abc{d}e"
    assert _expand("word {today} tail{{x}}") == "word 2026-07-09 tail{x}"
    assert _expand("plain tail") == "plain tail"
