"""Behavioural tests targeting mutation survivors in skit/cli.py (chunk 4/6).

These pin the read/print helpers and the small pure translators against wrong-text,
dropped-cell, and swapped-argument mutants:

- ``_print_declared_table`` — the exe/meta read table: column headers and per-row cells.
- ``_print_show_human`` — the ``skit show`` human view (the "Needs:" line).
- ``_print_candidate`` — the onboarding candidate print (the demotion warning).
- ``_refuse_unusable_add_flags`` — the add-flag refusal messages.
- ``_render_declared_warning`` — the closed edit_declared warning renderer.
- ``_reanchor_as_envdefault`` — the --normalize re-anchor (user decisions survive).

English catalog; assertions are on real rendered output / returned values through the
real CLI (CliRunner) or a direct call to the helper with a controlled console.
"""

from __future__ import annotations

import io
from pathlib import Path

import pytest
from rich.console import Console
from typer.testing import CliRunner

from skit import analysis, cli, i18n, store
from skit.params import ParamDecl

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    i18n.init("en")
    return tmp_path


def _norm(text: str) -> str:
    """Collapse Rich's terminal-width wrapping so a message matches as one line."""
    return " ".join(text.split())


@pytest.fixture
def captured_console(monkeypatch: pytest.MonkeyPatch) -> io.StringIO:
    """Redirect the module-level Console to a wide StringIO so table/candidate rendering
    is captured verbatim, without terminal-width wrapping."""
    buf = io.StringIO()
    monkeypatch.setattr(cli, "console", Console(file=buf, width=200))
    return buf


def _exe(tmp_path: Path, name: str = "prog") -> store.Entry:
    prog = tmp_path / "t"
    prog.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    prog.chmod(0o755)
    return store.add_exe(prog, name=name)


# --------------------------------------------------------------------------
# _print_declared_table — column headers
# --------------------------------------------------------------------------


def _header_cells(out: str) -> list[str]:
    """The header row's cells, in order, exactly as rendered (heavy vertical bars delimit
    the header line of a Rich table)."""
    header_line = next(line for line in out.splitlines() if "┃" in line)
    return [c.strip() for c in header_line.split("┃") if c.strip()]


def test_declared_table_headers_are_exact(captured_console: io.StringIO) -> None:
    """Every column header renders exactly its English label — a garbled msgid ("XX…XX"),
    a lower/upper-cased variant, or a swapped header is observable in the header row."""
    decls = [ParamDecl(name="width", delivery="flag", flag="--width", type="int", default=800)]
    cli._print_declared_table(decls, {})
    assert _header_cells(captured_console.getvalue()) == [
        "Parameter",
        "Delivery",
        "Type",
        "Default",
        "Secret",
        "Last value",
    ]


def test_declared_table_row_cells_render_each_value(captured_console: io.StringIO) -> None:
    """Each row cell carries its own value: the type, the default, the secret column (env
    source), and the remembered last value all appear. A dropped/None-ified cell or a
    last-value lookup keyed on the wrong name makes its value vanish."""
    decls = [
        ParamDecl(name="width", delivery="flag", flag="--width", type="int", default=800),
        ParamDecl(name="TOKEN", delivery="env", secret=True, env_source="MY_TOKEN"),
    ]
    cli._print_declared_table(decls, {"width": "1024"})
    out = captured_console.getvalue()
    assert "int" in out  # the Type cell of the width row
    assert "800" in out  # the Default cell of the width row
    assert "MY_TOKEN" in out  # the Secret cell ("yes ← $MY_TOKEN") of the TOKEN row
    assert "1024" in out  # the Last-value cell, looked up by the row's own name


# --------------------------------------------------------------------------
# _print_candidate — the onboarding demotion warning
# --------------------------------------------------------------------------


def test_print_candidate_demoted_warning_text(captured_console: io.StringIO) -> None:
    """A demoted candidate prints the loop-accumulator warning verbatim (the leading space
    from the indent proves the msgid was not garbled into 'XX⚠…parameterXX')."""
    cand = analysis.Candidate(
        binding="const", name="total", type="int", default=0, demoted=True, demotion="accumulator"
    )
    cli._print_candidate(1, cand)
    out = captured_console.getvalue()
    assert " ⚠ looks like a loop accumulator — probably not a parameter" in out


# --------------------------------------------------------------------------
# _print_show_human — the "Needs:" line
# --------------------------------------------------------------------------


def test_show_human_needs_line(tmp_path: Path) -> None:
    """`skit show` lists external commands joined by ', ' under a "Needs:" label. The
    leading-space anchor kills the garbled-label mutant; the ', ' separator kills the
    join-string mutant (two needs are required for the separator to render at all)."""
    _exe(tmp_path)
    store.update_needs("prog", ["alpha", "beta"])
    result = runner.invoke(cli.app, ["show", "prog"])
    assert result.exit_code == 0, result.output
    assert " Needs: alpha, beta" in _norm(result.output)


# --------------------------------------------------------------------------
# _refuse_unusable_add_flags — the refusal messages
# --------------------------------------------------------------------------


def _js(tmp_path: Path) -> Path:
    src = tmp_path / "a.js"
    src.write_text("console.log(1)\n", encoding="utf-8")
    return src


def _sh(tmp_path: Path) -> Path:
    src = tmp_path / "b.sh"
    src.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    return src


def test_refuse_npm_reference_dep_message(tmp_path: Path) -> None:
    """A reference-mode npm entry with --dep is refused with the exact reference-mode line."""
    result = runner.invoke(
        cli.app, ["add", str(_js(tmp_path)), "--ref", "--dep", "chalk", "--no-input"]
    )
    assert result.exit_code == 2
    assert _norm(result.output) == (
        "Reference-mode entries take no managed dependencies — they run from their own "
        "project. Add it as a copy, or drop --dep."
    )
    assert not store.list_entries()


def test_refuse_non_npm_dep_message(tmp_path: Path) -> None:
    """A non-npm kind (shell) with --dep is refused with the kind-specific line, %(kind)s
    substituted."""
    result = runner.invoke(cli.app, ["add", str(_sh(tmp_path)), "--dep", "jq", "--no-input"])
    assert result.exit_code == 2
    assert _norm(result.output) == "shell entries don't take package dependencies — drop --dep."
    assert not store.list_entries()


def test_refuse_python_constraint_message(tmp_path: Path) -> None:
    """A --python constraint on a non-uv kind is refused with the exact python-constraint line."""
    result = runner.invoke(cli.app, ["add", str(_js(tmp_path)), "--python", ">=3.11", "--no-input"])
    assert result.exit_code == 2
    assert _norm(result.output) == "A Python constraint doesn't apply to js scripts."
    assert not store.list_entries()


# --------------------------------------------------------------------------
# _render_declared_warning — the closed warning-code set
# --------------------------------------------------------------------------


def test_render_declared_warning_exact_line_per_code() -> None:
    """Every closed warning code renders exactly its English line with %(name)s filled in;
    a garbled msgid is observable as an unequal string."""
    expected = {
        "not-declared": "w isn't a declared parameter; skipped.",
        "already-declared": "w is already declared; skipped.",
        "bad-delivery": "w: that delivery isn't available for this kind; skipped.",
        "not-a-placeholder": (
            "w isn't a template placeholder, so it can't use placeholder delivery; skipped."
        ),
        "bad-type": "w: unknown type; skipped (use str, int, float, bool, or choice).",
        "bad-default": "w: the default doesn't fit its type; skipped.",
        "choice-without-choices": "w: a choice parameter needs choices; set --choices w=a,b,c.",
    }
    for code, line in expected.items():
        assert cli._render_declared_warning(f"{code}:w") == line


def test_render_declared_warning_splits_on_first_colon() -> None:
    """The code is everything before the FIRST colon and the name is the remainder — a name
    that itself contains a colon must round-trip whole (partition, not rpartition, which
    would misread the code and raise KeyError)."""
    assert cli._render_declared_warning("bad-type:na:me") == (
        "na:me: unknown type; skipped (use str, int, float, bool, or choice)."
    )


# --------------------------------------------------------------------------
# _reanchor_as_envdefault — user decisions survive the re-read
# --------------------------------------------------------------------------


def test_reanchor_preserves_secret_prompt_and_env_source() -> None:
    """The re-anchored decl takes binding/type/default from the freshly-read candidate but
    carries the user's own secret flag, custom prompt, and env source across unchanged."""
    cand = analysis.Candidate(
        binding="envdefault", name="FOO", type="str", default="bar", env_name="FOO"
    )
    spec = ParamDecl(name="FOO", secret=True, prompt="Custom prompt", env_source="MY_ENV")
    decl = cli._reanchor_as_envdefault(spec, cand)
    assert decl.secret is True
    assert decl.prompt == "Custom prompt"
    assert decl.env_source == "MY_ENV"
