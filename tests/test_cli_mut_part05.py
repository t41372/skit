"""Behavioural tests targeting mutation-testing survivors in skit/cli.py (chunk 5/6).

Covers four helpers: `_render_normalize_warning` (the shell --normalize refusal renderer),
`_require_file` (the add-path existence guard), `_resolve_npm_dependencies` (the js/ts
copy-add dependency resolver), and `_show_command_params` (the command-template read view).

Style matches tests/test_cli.py / tests/test_cli_mut.py: CliRunner is not needed here because
every target is a pure/near-pure helper — we call it directly and pin its real output (the exact
English copy, the masked-secret cell, the args it hands to Rich's Prompt.ask). Interactive
branches are driven by stubbing sys.stdin.isatty + Prompt.ask, the repo's established pattern.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from skit import argstate, cli, store
from skit.langs.registry import spec_for
from skit.params import ParamDecl

_ = argstate  # imported for parity with sibling files; state helpers used indirectly via store


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _js_file(
    tmp_path: Path, body: str = 'import chalk from "chalk";\n', name: str = "t.mjs"
) -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


def _js_scanner():
    spec = spec_for("js")
    assert spec is not None
    scanner = spec.dep_scanner
    assert scanner is not None
    return scanner


# ==========================================================================
# _render_normalize_warning — the "code:name" -> user line renderer
# ==========================================================================


def test_render_normalize_not_a_const_exact():
    # kills the XX-wrap mutant on the not-a-const entry.
    out = cli._render_normalize_warning("not-a-const:V")
    assert out == (
        "V isn't a plain constant with a literal value, so there's nothing to normalize; skipped."
    )


def test_render_normalize_multiple_assignments_exact():
    # kills both the XX-wrap and the "Skipped." -> "skipped." case mutant.
    out = cli._render_normalize_warning("multiple-assignments:V")
    assert out == (
        "V is assigned more than once at the top level; "
        "normalizing it would change which value wins. Skipped."
    )
    assert out.endswith("wins. Skipped.")


def test_render_normalize_readonly_exact():
    out = cli._render_normalize_warning("readonly:V")
    assert out == (
        "V is readonly, so the script could never take a value from the environment; skipped."
    )


def test_render_normalize_already_env_exact():
    out = cli._render_normalize_warning("already-env:V")
    assert out == "V already reads from the environment; nothing to do."


def test_render_normalize_unsafe_literal():
    # unsafe-literal is two concatenated string literals; pin both halves.
    out = cli._render_normalize_warning("unsafe-literal:V")
    assert "XX" not in out  # kills the XX-wrap mutants on either half
    assert out.startswith("V's value contains a character that can't be moved into ")
    assert "or a newline); skipped" in out  # kills the UPPERCASE mutant on the second half


def test_render_normalize_syntax_error_exact():
    # kills the XX-wrap and the two case mutants on the syntax-error entry.
    out = cli._render_normalize_warning("syntax-error:V")
    assert out == "Could not parse the script (syntax error); nothing was normalized."


def test_render_normalize_splits_code_on_first_colon():
    # A `--normalize FOO:BAR` typo on a syntactically-broken script reaches the renderer as
    # "syntax-error:FOO:BAR" (normalize.py refuses every requested name verbatim). The code is
    # the part before the FIRST colon; rpartition would read code="syntax-error:FOO" -> KeyError.
    out = cli._render_normalize_warning("syntax-error:FOO:BAR")
    assert out == "Could not parse the script (syntax error); nothing was normalized."


# ==========================================================================
# _require_file — the add-path existence guard (kind-neutral since the prompt kind)
# ==========================================================================


def test_require_file_missing_names_the_path(tmp_path: Path):
    missing = tmp_path / "ghost.py"
    with pytest.raises(store.StoreError) as excinfo:
        cli._require_file(missing)
    msg = str(excinfo.value)
    assert "XX" not in msg  # kills the XX-wrapped msgid mutant
    assert msg == f"File not found: {missing}"  # kills str(None) — the real path must be named


def test_require_file_existing_does_not_raise(tmp_path: Path):
    present = tmp_path / "real.py"
    present.write_text("print(1)\n", encoding="utf-8")
    cli._require_file(present)  # must not raise for an existing file


def test_require_file_directory_names_the_path_in_not_a_file(tmp_path: Path):
    # The second arm (exists but isn't a file — a directory) has its own msgid; exact
    # match kills the XX-wrap and the str(None) path substitution.
    with pytest.raises(store.StoreError) as excinfo:
        cli._require_file(tmp_path)
    assert str(excinfo.value) == f"Not a file: {tmp_path}"


# ==========================================================================
# _require_exists — the exe lane's existence guard (any existing path, dirs included)
# ==========================================================================


def test_require_exists_missing_names_the_path(tmp_path: Path):
    # The exe twin of _require_file: only the "not found" arm exists. Exact whole-message
    # match kills the XX-wrapped msgid and the str(None) path substitution.
    missing = tmp_path / "ghost.bin"
    with pytest.raises(store.StoreError) as excinfo:
        cli._require_exists(missing)
    msg = str(excinfo.value)
    assert "XX" not in msg
    assert msg == f"File not found: {missing}"


def test_require_exists_accepts_a_directory(tmp_path: Path):
    # add_exe's broader contract: any existing path passes, directories included — the guard
    # must NOT raise here (it has no "is_file" arm, unlike _require_file).
    cli._require_exists(tmp_path)


# ==========================================================================
# _resolve_npm_dependencies — js/ts copy-add dependency resolution
# ==========================================================================


def test_resolve_npm_invalid_utf8_reads_with_replace(tmp_path: Path):
    # A file that carries a chalk import plus a stray invalid-UTF-8 byte. errors="replace" must
    # decode it (replacing the bad byte) so the scanner still finds "chalk"; a strict decode
    # (errors=None / dropped errors kwarg) raises UnicodeDecodeError, and a bogus handler name
    # ("XXreplaceXX" / "REPLACE") raises LookupError — either escapes the OSError-only guard.
    bad = tmp_path / "bad.mjs"
    bad.write_bytes(b'import chalk from "chalk";\n// \xff\xfe stray\n')
    assert cli._resolve_npm_dependencies(bad, None, True, _js_scanner()) == ["chalk"]


def test_resolve_npm_no_input_skips_prompt_even_on_a_tty(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    # no_input short-circuits to the scanned suggestions WITHOUT prompting, even on a tty
    # (the `no_input or not _is_interactive()` gate). An `and` mutant would fall through to Prompt.ask.
    src = _js_file(tmp_path)
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)

    def boom(*a: object, **k: object) -> object:
        raise AssertionError("Prompt.ask must not run under no_input")

    monkeypatch.setattr(cli.Prompt, "ask", boom)
    assert cli._resolve_npm_dependencies(src, None, True, _js_scanner()) == ["chalk"]


def test_resolve_npm_interactive_prompt_wiring(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    # Interactive path: pin exactly what the resolver hands Prompt.ask — the prompt copy, the
    # suggestion default, and the shared console — and that accepting the default records it.
    src = _js_file(tmp_path)
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)
    calls: list[tuple[tuple[object, ...], dict[str, object]]] = []

    def spy(*a: object, **k: object) -> object:
        calls.append((a, k))
        return k.get("default", "")

    monkeypatch.setattr(cli.Prompt, "ask", spy)
    result = cli._resolve_npm_dependencies(src, None, False, _js_scanner())
    assert result == ["chalk"]  # default accepted -> the scanned suggestion is recorded
    assert len(calls) == 1
    args, kwargs = calls[0]
    assert args[:1] == (
        "Dependencies to install (Enter to accept, edit the list, or '-' for none)",
    )
    assert kwargs.get("default") == "chalk"
    assert kwargs.get("console") is cli.console


def test_resolve_npm_interactive_none_declines(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    # Answering "none" (case-insensitively) clears the list. The `.lower()` and the literal
    # "none" in the sentinel tuple both matter: an .upper() or a case-shifted "none" would miss
    # and try to install a package literally named "none".
    src = _js_file(tmp_path)
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: "none")
    assert cli._resolve_npm_dependencies(src, None, False, _js_scanner()) == []


# ==========================================================================
# _show_command_params — the command-template read view
# ==========================================================================


def test_show_command_params_secret_placeholder_is_masked(tmp_path: Path, capsys):
    # A declared-secret placeholder with a stored value must show the mask, never the value:
    # the `d.secret if d is not None else False` argument decides masking.
    entry = store.add_command("echo {token}", name="cmdsec")
    declared = [ParamDecl(name="token", delivery="placeholder", secret=True)]
    cli._show_command_params(entry, declared, {"token": "s3cr3tval"})
    out = capsys.readouterr().out
    assert "•••" in out
    assert "s3cr3tval" not in out


def test_show_command_params_placeholder_schema_suffix(tmp_path: Path, capsys):
    # A declared placeholder contributes an inline schema marker; passing None instead of the
    # decl drops it, so the type/default disappear.
    entry = store.add_command("echo {count}", name="cmdsch")
    declared = [ParamDecl(name="count", delivery="placeholder", type="int", default=42)]
    cli._show_command_params(entry, declared, {})
    out = capsys.readouterr().out
    assert "count = " in out
    assert "default 42" in out


def test_show_command_params_env_riders(tmp_path: Path, capsys):
    # A command with no placeholders but two declared env riders: the header, each rider's
    # last-value cell (masked for the secret), and each rider's schema suffix must all render.
    entry = store.add_command("echo hi", name="cmdenv")
    declared = [
        ParamDecl(name="REGION", delivery="env", type="str", default="us-east-1"),
        ParamDecl(name="API_KEY", delivery="env", secret=True),
    ]
    last = {"REGION": "eu-west", "API_KEY": "abc123"}
    cli._show_command_params(entry, declared, last)
    out = capsys.readouterr().out
    # The entry has no placeholders, so the env-rider header is the first line printed;
    # startswith (not `in`) so an XX-wrapped "XX...XX" header can't match as a substring.
    assert out.startswith("Declared environment variables (set on the run):")
    assert "eu-west" in out  # REGION's last value, keyed by d.name (not None)
    assert "•••" in out  # API_KEY masked via d.secret (not None/False)
    assert "abc123" not in out  # the secret value is never echoed
    assert "default us-east-1" in out  # REGION's schema suffix (decl, not None)


def test_show_command_params_env_delivered_placeholder_not_duplicated(tmp_path: Path, capsys):
    # An env-delivered decl whose name IS a placeholder belongs to the placeholders section only:
    # the `delivery == "env" and name not in placeholders` filter excludes it from env riders.
    # An `or` there would resurrect it as a duplicate env-rider row.
    entry = store.add_command("echo {name}", name="cmddup")
    declared = [ParamDecl(name="name", delivery="env")]
    cli._show_command_params(entry, declared, {})
    out = capsys.readouterr().out
    assert "Command template placeholders (the run form asks for them):" in out
    assert "Declared environment variables (set on the run):" not in out
