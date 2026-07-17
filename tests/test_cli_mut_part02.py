"""Mutation-kill tests for src/skit/cli.py (chunk 2/6).

Pins observable behaviour of the read-view render helpers (`_declared_default_cell`,
`_declared_last_value`, `_declared_schema_suffix`), the onboarding default-selection
string (`_default_selection`), the `skit deps NAME` read view (`_deps_read_view`, via the
real CLI), and the doctor drift scan (`_drifted_entries`).

Render helpers are pure functions the read table/command-view render through, so they are
called directly (mirroring test_cli.py's "call the module functions directly" convention);
`_deps_read_view` is exercised end-to-end through `skit deps` with CliRunner; message copy
is asserted against the English catalog (SKIT_LANG=en).
"""

from __future__ import annotations

import dataclasses
from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import analysis, cli, store
from skit.langs.python import metawriter
from skit.langs.registry import spec_for
from skit.params import ParamDecl

runner = CliRunner()


@pytest.fixture(autouse=True)
def tmp_store(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("SKIT_DATA_DIR", str(tmp_path / "data"))
    monkeypatch.setenv("SKIT_STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path / "config"))
    monkeypatch.setenv("SKIT_LANG", "en")
    return tmp_path


def _py(tmp_path: Path, body: str, name: str = "job.py") -> Path:
    p = tmp_path / name
    p.write_text(body, encoding="utf-8")
    return p


# --------------------------------------------------------------------------
# _declared_default_cell
# --------------------------------------------------------------------------


def test_declared_default_cell_none_is_dash():
    # No default -> the em-dash placeholder, verbatim (kills is-None inversion + "XX—XX" wrap).
    assert cli._declared_default_cell(ParamDecl(name="X", type="str", default=None)) == "—"


def test_declared_default_cell_public_shows_value():
    # A public default renders its actual value, not str(None) and not the "—" placeholder.
    assert (
        cli._declared_default_cell(ParamDecl(name="X", type="str", default="hello", secret=False))
        == "hello"
    )


def test_declared_default_cell_secret_is_masked():
    # A secret default is masked with the bullet glyph, never echoed (kills "XX•••XX" wrap).
    assert (
        cli._declared_default_cell(ParamDecl(name="X", type="str", default="hello", secret=True))
        == "•••"
    )


# --------------------------------------------------------------------------
# _declared_last_value
# --------------------------------------------------------------------------


def test_declared_last_value_secret_present_is_masked():
    # A stored secret is masked, never echoed; the in-dict check must be positive (kills the
    # bullet wrap and the `name in last` -> `not in` inversion).
    assert cli._declared_last_value("API", True, {"API": "plaintext"}) == "•••"


def test_declared_last_value_secret_absent_is_dash():
    # No stored value for a secret -> the em-dash placeholder (kills the else-"XX—XX" wrap and,
    # again, the membership inversion which would wrongly mask an absent value).
    assert cli._declared_last_value("API", True, {}) == "—"


# --------------------------------------------------------------------------
# _declared_schema_suffix
# --------------------------------------------------------------------------


def test_declared_schema_suffix_public_default_optional():
    # type · default · optional, rendered exactly (kills the default-branch inversion, the
    # shown=None mutant, the "XXdefaultXX"/"XXoptionalXX" wraps, and the " · " separator wrap).
    d = ParamDecl(name="X", type="str", default="v", required=False, secret=False)
    assert cli._declared_schema_suffix(d) == "  [dim]str · default v · optional[/dim]"


def test_declared_schema_suffix_secret_default_and_secret_flag():
    # A secret default is masked in the suffix and the trailing "secret" flag is verbatim
    # (kills the gettext(None) mutant, the "XX•••XX" wrap, and the "secret"/"SECRET" mutants).
    d = ParamDecl(name="X", type="str", default="v", required=False, secret=True)
    assert cli._declared_schema_suffix(d) == "  [dim]str · default ••• · optional · secret[/dim]"


# --------------------------------------------------------------------------
# _default_selection
# --------------------------------------------------------------------------


def test_default_selection_mixed_joins_clean_indices_with_comma():
    # Some clean, some demoted -> a comma-joined 1-based index list (kills the "XX,XX" separator).
    cands = [
        analysis.Candidate(binding="const", name="a"),
        analysis.Candidate(binding="const", name="b", demoted=True),
        analysis.Candidate(binding="const", name="c"),
    ]
    assert cli._default_selection(cands) == "1,3"


# --------------------------------------------------------------------------
# _deps_read_view  (via `skit deps NAME`)
# --------------------------------------------------------------------------


def _lines(output: str) -> list[str]:
    return [line.strip() for line in output.splitlines()]


def test_deps_read_view_lists_deps_python_and_needs(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    runner.invoke(
        cli.app,
        [
            "deps",
            "a",
            "--dep",
            "requests",
            "--dep",
            "rich",
            "--python",
            ">=3.11",
            "--need",
            "jq",
            "--need",
            "fzf",
        ],
    )
    result = runner.invoke(cli.app, ["deps", "a"])
    assert result.exit_code == 0, result.output
    lines = _lines(result.output)
    # Exact lines kill the msgid XX-wraps, the case flips, and the ", " join-separator wraps.
    assert "Dependencies of a: requests, rich" in lines
    assert "Python constraint: >=3.11" in lines
    assert "External commands needed by a: jq, fzf" in lines


def test_deps_read_view_empty_deps_and_needs_show_dash(tmp_path):
    # A fresh python entry: deps + needs both empty -> em-dash placeholder on each line
    # (kills the `or "XX—XX"` fallbacks on the deps and needs join expressions).
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a"])
    assert result.exit_code == 0, result.output
    lines = _lines(result.output)
    assert "Dependencies of a: —" in lines
    assert "External commands needed by a: —" in lines


def test_deps_read_view_json_preserves_unicode(tmp_path):
    # The --json contract keeps non-ASCII verbatim (never \\u-escaped) for a needed command.
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    runner.invoke(cli.app, ["deps", "a", "--need", "café"])
    result = runner.invoke(cli.app, ["deps", "a", "--json"])
    assert result.exit_code == 0, result.output
    assert "café" in result.output
    assert "\\u" not in result.output


# --------------------------------------------------------------------------
# _drifted_entries  (the doctor batch reconcile)
# --------------------------------------------------------------------------


def _ok_entry(tmp_path: Path) -> store.Entry:
    # A python entry whose in-file [tool.skit] param matches the source: has specs, no drift.
    text = metawriter.write_params(
        'A = "x"\nprint(A)\n', [ParamDecl(name="A", binding="const", type="str", default="x")]
    )
    return store.add_python(_py(tmp_path, text, name="ok.py"), name="okentry")


def _drift_entry(tmp_path: Path) -> store.Entry:
    # A declared param for a variable that isn't in the source -> reconcile reports drift.
    text = metawriter.write_params(
        "print(1)\n", [ParamDecl(name="GHOST", binding="const", type="str", default="x")]
    )
    return store.add_python(_py(tmp_path, text, name="drift.py"), name="driftentry")


def test_drifted_entries_reports_only_drifting_entries(tmp_path):
    # An entry WITH specs but NO drift must not be reported (kills `specs and` -> `specs or`,
    # which would append every entry that merely has specs).
    assert cli._drifted_entries([_ok_entry(tmp_path)]) == []


def test_drifted_entries_detects_actual_drift(tmp_path):
    assert cli._drifted_entries([_drift_entry(tmp_path)]) == ["driftentry"]


def test_drifted_entries_skips_and_continues_past_skippable(tmp_path):
    # A skippable entry (command: no analyzer/params_io) precedes a drifting one. The loop must
    # `continue` past it and still reach the drift (kills `continue` -> `break`).
    skip = store.add_command("echo hi", name="cmdentry")
    drift = _drift_entry(tmp_path)
    assert cli._drifted_entries([skip, drift]) == ["driftentry"]


def test_drifted_entries_skips_unknown_kind_without_crashing(tmp_path):
    # An entry of a kind this skit doesn't know (spec_for -> None) is skipped cleanly. The
    # short-circuit on `spec is None` must stay an OR (kills `spec is None and spec.analyzer...`,
    # which dereferences None.analyzer and crashes).
    real = _ok_entry(tmp_path)
    unknown = dataclasses.replace(real, meta=dataclasses.replace(real.meta, kind="future-kind"))
    assert spec_for("future-kind") is None
    assert cli._drifted_entries([unknown]) == []


def test_drifted_entries_skips_analyzer_degraded_spec(tmp_path, monkeypatch):
    # The documented grammar-degraded state: analyzer is None but params_io still reads specs.
    # The guard must skip on `analyzer is None` alone (kills `analyzer is None and params_io is
    # None`, which would proceed to call None.reconcile and crash).
    entry = _ok_entry(tmp_path)
    py_spec = spec_for("python")
    assert py_spec is not None
    degraded = dataclasses.replace(py_spec, analyzer=None)
    assert degraded.params_io is not None
    monkeypatch.setattr(cli, "spec_for", lambda kind: degraded)
    assert cli._drifted_entries([entry]) == []
