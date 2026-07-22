"""Context-aware POSIX quoting for command templates (Fix A in langs/launch.py).

Old behavior blindly ``shlex.quote``-d every ``{placeholder}`` value regardless of the
surrounding shell context, so a value like ``$(printf unexpected)`` dropped inside a
``"double-quoted"`` template slot stayed *live* — the reproduced bug. TemplateLaunch now
tracks the shell quote state across the template's own text and escapes each value FOR the
position its placeholder sits in.

Two layers of coverage:
  * the pure helpers ``_posix_quote_state`` / ``_posix_quote_value``, pinned with exact-string
    assertions (the mutation-kill idiom of tests/test_launch_mut.py);
  * the whole substitution driven through ``build_command`` / ``describe_command``, plus real
    ``sh -c`` execution proving the user-visible result.

Store/locale isolation comes from conftest's autouse fixtures (English catalog, per-test
tmp store), so no per-file fixture is needed.
"""

from __future__ import annotations

import shlex
import subprocess
import sys

import pytest

from skit.langs import launch

_POSIX_ONLY = pytest.mark.skipif(sys.platform == "win32", reason="POSIX shell quoting/execution")


# ==========================================================================
# _posix_quote_state — the quote-context state machine (exact transitions)
# ==========================================================================


def test_state_open_and_close_single_quote():
    assert launch._posix_quote_state("'", "") == "'"  # a bare ' opens single-quote context
    assert launch._posix_quote_state("'", "'") == ""  # the matching ' closes it


def test_state_open_and_close_double_quote():
    assert launch._posix_quote_state('"', "") == '"'  # a bare " opens double-quote context
    assert launch._posix_quote_state('"', '"') == ""  # the matching " closes it


def test_state_backslash_skips_next_char_in_unquoted_so_quote_stays_shut():
    # Outside single quotes a backslash consumes the following char, so `\"` / `\'` do NOT open
    # a quote context (they are an escaped literal quote).
    assert launch._posix_quote_state('\\"', "") == ""
    assert launch._posix_quote_state("\\'", "") == ""
    # And having consumed the char, a subsequent real quote still opens as normal.
    assert launch._posix_quote_state("\\a'", "") == "'"


def test_state_backslash_skips_closing_quote_inside_double():
    # Inside "..." an escaped `\"` does not close the context...
    assert launch._posix_quote_state('\\"', '"') == '"'
    # ...but the next *bare* " does.
    assert launch._posix_quote_state('\\""', '"') == ""


def test_state_backslash_is_literal_inside_single_quotes():
    # POSIX single quotes have no escapes: a backslash is an ordinary literal that does NOT
    # consume the following char, so the very next ' still closes the context.
    assert launch._posix_quote_state("\\'", "'") == ""
    # A lone backslash inside single quotes therefore leaves the context open.
    assert launch._posix_quote_state("\\", "'") == "'"


def test_state_the_other_quote_kind_is_literal():
    # A " inside '...' is a literal (no state change); a ' inside "..." likewise.
    assert launch._posix_quote_state('"', "'") == "'"
    assert launch._posix_quote_state("'", '"') == '"'


def test_state_carries_across_successive_chunks():
    # Quote-neutral text leaves whatever context we were handed untouched (the between-token
    # carry that _substitute_posix relies on)...
    assert launch._posix_quote_state("plain text", "'") == "'"
    assert launch._posix_quote_state("plain text", '"') == '"'
    assert launch._posix_quote_state("plain text", "") == ""
    # ...and a later chunk can close a context an earlier chunk opened.
    opened = launch._posix_quote_state("open '", "")
    assert opened == "'"
    assert launch._posix_quote_state("close '", opened) == ""


def test_state_dangling_backslash_pends_across_the_boundary():
    # A chunk ENDING on an unconsumed backslash reports it: that escape applies to the first
    # character the caller emits next, so swallowing it here would hand the escape to the
    # substituted value (the `"foo\{name}"` hole _substitute_posix neutralizes).
    assert launch._posix_quote_state("foo\\", "") == "\\"
    assert launch._posix_quote_state("\\", '"') == '"\\'
    # An EVEN run of backslashes self-consumes: nothing pends.
    assert launch._posix_quote_state("foo\\\\", "") == ""
    assert launch._posix_quote_state("\\\\", '"') == '"'
    # Inside single quotes a backslash is literal — never pending.
    assert launch._posix_quote_state("foo\\", "'") == "'"


def test_state_resumes_a_pending_backslash_by_consuming_the_first_char():
    # Resuming with pending state: the first character is the escaped one — even a quote
    # character, which therefore must NOT open a context.
    assert launch._posix_quote_state("x", '"\\') == '"'
    assert launch._posix_quote_state("'abc", "\\") == ""
    assert launch._posix_quote_state('"abc', "\\") == ""
    # An empty resumed chunk keeps the escape pending.
    assert launch._posix_quote_state("", '"\\') == '"\\'
    assert launch._posix_quote_state("", "\\") == "\\"
    # Resume consumes EXACTLY one char as escaped, then processes the rest: a two-char
    # resume where the second char is a real quote must still act on it (i=1, not i=2 or
    # more). Here the escaped 'x' is skipped and the bare '"' closes the double context.
    assert launch._posix_quote_state('x"', '"\\') == ""
    # The unquoted twin: escaped 'x' skipped, then '"' OPENS a double context.
    assert launch._posix_quote_state('x"', "\\") == '"'


# ==========================================================================
# _posix_quote_value — position-aware escaping (exact output + escape ORDER)
# ==========================================================================


def test_value_single_context_escapes_embedded_apostrophe():
    # Inside '...', an embedded ' becomes '\'' (close, escaped-quote, reopen).
    assert launch._posix_quote_value("a'b", "'") == "a'\\''b"


def test_value_single_context_plain_value_is_verbatim():
    assert launch._posix_quote_value("plain", "'") == "plain"


def test_value_double_context_escapes_backslash():
    assert launch._posix_quote_value("\\", '"') == "\\\\"


def test_value_double_context_escapes_double_quote():
    assert launch._posix_quote_value('"', '"') == '\\"'


def test_value_double_context_escapes_dollar():
    assert launch._posix_quote_value("$", '"') == "\\$"


def test_value_double_context_escapes_backtick():
    assert launch._posix_quote_value("`", '"') == "\\`"


def test_value_double_context_neutralizes_command_substitution():
    # The reproduced bug: $(...) must arrive dead, not live, inside double quotes.
    assert launch._posix_quote_value("$(printf unexpected)", '"') == "\\$(printf unexpected)"


def test_value_double_context_backslash_doubling_precedes_dollar_escape():
    # Value is literally backslash-dollar (`\$`). Backslash-doubling MUST run first: otherwise
    # the backslash the $->\$ step injects would itself get doubled. Correct result is `\\\$`
    # (backslash-backslash = one literal backslash, then \$ = one literal dollar).
    assert launch._posix_quote_value("\\$", '"') == "\\\\\\$"


def test_value_double_context_backslash_before_double_quote_order():
    # A value carrying both \ and ": backslash-doubling must precede the "-escape, or the
    # backslash the "-escape injects would itself be doubled. \" -> \\ + \" (doubled backslash,
    # then the escaped quote), never \\\\ + ".
    assert launch._posix_quote_value('\\"', '"') == "\\\\" + '\\"'


def test_value_double_context_backtick_after_dollar():
    # A value carrying both $ and ` — both are neutralized, order-independently here.
    assert launch._posix_quote_value("$x`y`", '"') == "\\$x\\`y\\`"


def test_value_unquoted_context_defers_to_shlex_quote():
    assert launch._posix_quote_value("a b", "") == shlex.quote("a b")
    assert launch._posix_quote_value("a b", "") == "'a b'"
    assert launch._posix_quote_value("$(id)", "") == "'$(id)'"


# ==========================================================================
# _substitute_posix via build_command / describe_command / real execution
# ==========================================================================


def _run_sh(command: str | list[str]) -> subprocess.CompletedProcess[str]:
    # build_command returns list[str] | str; a command entry always renders a shell string —
    # narrow it here so every caller stays terse. Absolute /bin/sh (not bare "sh") keeps ruff's
    # S607 quiet and is guaranteed present under the _POSIX_ONLY guard these callers all carry.
    assert isinstance(command, str)
    return subprocess.run(["/bin/sh", "-c", command], capture_output=True, text=True, check=False)


@_POSIX_ONLY
def test_double_quoted_placeholder_neutralizes_command_substitution():
    from skit import launcher, store

    entry = store.add_command('printf "%s\\n" "{value}"', name="dq-cmdsub")
    cmd = launcher.build_command(entry, values={"value": "$(printf unexpected)"})
    assert isinstance(cmd, str)
    # The $ is backslash-escaped INSIDE the double quotes (old code left it live).
    assert cmd == 'printf "%s\\n" "\\$(printf unexpected)"'
    # The user-visible proof: the child prints the value literally, no substitution.
    result = _run_sh(cmd)
    assert result.returncode == 0
    assert result.stdout == "$(printf unexpected)\n"


@_POSIX_ONLY
def test_single_quoted_placeholder_stays_literal_with_apostrophe_and_substitution():
    from skit import launcher, store

    entry = store.add_command("echo '{v}'", name="sq-literal")
    cmd = launcher.build_command(entry, values={"v": "a'b $(id)"})
    assert isinstance(cmd, str)
    assert cmd == "echo 'a'\\''b $(id)'"
    result = _run_sh(cmd)
    assert result.returncode == 0
    assert result.stdout == "a'b $(id)\n"


@_POSIX_ONLY
def test_unquoted_placeholder_embedded_in_a_word():
    from skit import launcher, store

    entry = store.add_command("echo scale={width}:-1", name="unq-word")
    cmd = launcher.build_command(entry, values={"width": "640"})
    assert cmd == "echo scale=640:-1"
    result = _run_sh(cmd)
    assert result.returncode == 0
    assert result.stdout == "scale=640:-1\n"


@_POSIX_ONLY
def test_unquoted_placeholder_hostile_value_cannot_escape_the_word():
    from skit import launcher, store

    entry = store.add_command("echo scale={width}:-1", name="unq-hostile")
    cmd = launcher.build_command(entry, values={"width": "640 $(id)"})
    # shlex.quote wraps the whole value, so the space and $(...) stay inside one word.
    assert cmd == "echo scale='640 $(id)':-1"
    result = _run_sh(cmd)
    assert result.returncode == 0
    assert result.stdout == "scale=640 $(id):-1\n"  # $(id) never ran


@_POSIX_ONLY
def test_unfilled_placeholder_travels_through_unchanged():
    from skit import launcher, store

    entry = store.add_command("echo {leftover}", name="unfilled")
    # Detach the placeholder from meta.params so _render's missing-value gate does not fire;
    # this isolates _substitute_posix's "name not in vals -> emit the original token" branch.
    entry.meta.params = None
    cmd = launcher.build_command(entry, values={})
    assert cmd == "echo {leftover}"


@_POSIX_ONLY
def test_brace_escapes_collapse_inside_quotes_without_disturbing_state():
    from skit import launcher, store

    entry = store.add_command('echo "{{x}} {v}"', name="braces-in-quotes")
    cmd = launcher.build_command(entry, values={"v": "$X"})
    # {{ }} collapse to literal single braces even inside the double quotes...
    assert cmd == 'echo "{x} \\$X"'
    assert "{{x}}" not in cmd
    # ...and the intervening braces are state-neutral: {v} still gets DOUBLE-context escaping
    # (the \$X above), proving the escape tokens did not reset the quote state.
    result = _run_sh(cmd)
    assert result.returncode == 0
    assert result.stdout == "{x} $X\n"


@_POSIX_ONLY
def test_substituted_value_containing_double_braces_is_not_rescanned():
    from skit import launcher, store

    entry = store.add_command('echo "{v}"', name="one-pass")
    cmd = launcher.build_command(entry, values={"v": "{{x}}"})
    # One-pass substitution: the value's own "{{" is NOT treated as a template escape.
    assert cmd == 'echo "{{x}}"'
    result = _run_sh(cmd)
    assert result.returncode == 0
    assert result.stdout == "{{x}}\n"


@_POSIX_ONLY
def test_extra_args_are_appended_shell_quoted_after_the_template():
    from skit import launcher, store

    entry = store.add_command("echo {v}", name="extra-args")
    cmd = launcher.build_command(entry, ["a b", "$X"], values={"v": "hi"})
    assert cmd == "echo hi " + shlex.join(["a b", "$X"])
    result = _run_sh(cmd)
    assert result.returncode == 0
    assert result.stdout == "hi a b $X\n"  # extra args quoted, $X not expanded


@_POSIX_ONLY
def test_quote_state_affects_only_later_placeholders():
    from skit import launcher, store

    entry = store.add_command('echo "{a}" {b}', name="two-slots")
    cmd = launcher.build_command(entry, values={"a": "$A", "b": "$B"})
    # {a} sits in double quotes (backslash-escaped $), {b} lands unquoted after the closing
    # quote (shlex single-quoting): different escaping proves the first slot's context did not
    # leak into the second.
    assert cmd == "echo \"\\$A\" '$B'"
    result = _run_sh(cmd)
    assert result.returncode == 0
    assert result.stdout == "$A $B\n"


@_POSIX_ONLY
def test_describe_command_uses_the_same_context_aware_quoting():
    from skit import launcher, store

    entry = store.add_command('echo "{v}"', name="desc-context")
    line = launcher.describe_command(entry, values={"v": "$(id)"})
    # The transparency/dry-run line escapes the value for its double-quoted slot too.
    assert line == 'echo "\\$(id)"'


@_POSIX_ONLY
def test_dangling_backslash_before_a_placeholder_cannot_eat_the_value_escape():
    from skit import launcher, store

    # A template backslash immediately before a double-quoted placeholder would otherwise
    # consume the `\` guarding the value's `$`, re-arming command substitution. The renderer
    # completes the template's backslash into a literal `\\` pair, keeping the escape intact.
    entry = store.add_command('printf "%s\\n" "foo\\{name}"', name="dangling-dq")
    cmd = launcher.build_command(entry, values={"name": "$(printf pwned)"})
    assert cmd == 'printf "%s\\n" "foo\\\\\\$(printf pwned)"'
    result = _run_sh(cmd)
    assert result.returncode == 0
    # The author's backslash survives as a literal; the substitution stays dead.
    assert result.stdout == "foo\\$(printf pwned)\n"


@_POSIX_ONLY
def test_dangling_backslash_in_unquoted_position_is_neutralized_too():
    from skit import launcher, store

    entry = store.add_command("printf %s\\\\n foo\\{name}", name="dangling-unq")
    cmd = launcher.build_command(entry, values={"name": "$(printf pwned)"})
    # The completed `\\` is a literal backslash; the value keeps its own single quotes.
    assert cmd == "printf %s\\\\n foo\\\\'$(printf pwned)'"
    result = _run_sh(cmd)
    assert result.returncode == 0
    assert result.stdout == "foo\\$(printf pwned)\n"


@_POSIX_ONLY
def test_even_backslash_run_before_a_placeholder_adds_no_neutralizer():
    from skit import launcher, store

    # `\\` before the slot is a self-contained escaped backslash — no pending escape, so the
    # renderer must NOT add another one (that would grow the backslash run and re-arm the $).
    entry = store.add_command('printf "%s\\n" "a\\\\{name}"', name="even-run")
    cmd = launcher.build_command(entry, values={"name": "$(printf pwned)"})
    assert cmd == 'printf "%s\\n" "a\\\\\\$(printf pwned)"'
    result = _run_sh(cmd)
    assert result.stdout == "a\\$(printf pwned)\n"


@_POSIX_ONLY
def test_dangling_backslash_before_brace_escape_and_unfilled_placeholder_is_absorbed():
    from skit import launcher, store

    # A `{{` emission (and an unfilled `{name}`) begins with a brace: the template's dangling
    # backslash escapes THAT — `\{` and `\\{` are the same literal two characters in sh — so
    # no neutralizer is added, and the state resolves for everything that follows.
    entry = store.add_command('printf "%s\\n" "\\{{x}} {later}"', name="dangling-brace")
    cmd = launcher.build_command(entry, values={"later": "$(printf pwned)"})
    assert cmd == 'printf "%s\\n" "\\{x} \\$(printf pwned)"'
    result = _run_sh(cmd)
    assert result.stdout == "\\{x} $(printf pwned)\n"
    # The unfilled twin, driven through the substitutor directly (a command entry validates
    # missing params before rendering, so an unfilled slot can't be staged through build):
    # `{never}` stays as-is, its brace absorbs the dangling escape, and the LATER filled
    # placeholder still gets clean double-quote-context escaping.
    rendered = launch.TemplateLaunch._substitute_posix('"\\{never} {later}"', {"later": "$(x)"})
    assert rendered == '"\\{never} \\$(x)"'


# ==========================================================================
# _render — the win32 branch (list2cmdline), driven by a faked platform so the
# `repl` closure and cmd.exe quoting stay covered on POSIX CI.
# ==========================================================================


def test_render_win32_uses_list2cmdline_not_posix_quoting(monkeypatch):
    from skit import launcher, store

    entry = store.add_command("echo {v}", name="win-space")
    monkeypatch.setattr("sys.platform", "win32")
    cmd = launcher.build_command(entry, ["c d"], values={"v": "a b"})
    assert isinstance(cmd, str)
    # Windows list2cmdline wraps a spaced value AND spaced extra args in double quotes; POSIX
    # would have single-quoted them.
    assert cmd == 'echo "a b" "c d"'
    assert "'a b'" not in cmd


def test_render_win32_repl_handles_brace_escapes_and_unfilled_placeholders(monkeypatch):
    from skit import launcher, store

    entry = store.add_command("echo {{x}} {filled} {unfilled}", name="win-repl")
    entry.meta.params = None  # let {unfilled} pass the missing-value gate
    monkeypatch.setattr("sys.platform", "win32")
    cmd = launcher.build_command(entry, values={"filled": "v"})
    # Every repl branch: {{ -> {, }} -> }, a filled placeholder, and an untouched one.
    assert cmd == "echo {x} v {unfilled}"
