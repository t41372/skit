"""Mutation-kill tests for skit.analysis.

Pins the exact user-facing copy emitted by the two render helpers (`drift_lines`,
`render_warning`) and two behavioural decisions (edit_specs' `resync` default, reconcile's
env-coverage bookkeeping). Every assertion here exercises a real reconcile/render code path
through the public language shims, mirroring tests/test_reconcile.py and
tests/test_shell_analyzer.py. The conftest pins SKIT_LANG=en and resets the i18n catalog per
test, so the English msgids are the observed output verbatim.
"""

from __future__ import annotations

import pytest

from skit import analysis
from skit.langs.python import reconcile as pyrec
from skit.langs.python.analyzer import analyze as py_analyze
from skit.langs.shell import analyzer as shell
from skit.params import ParamDecl

PY_SCRIPT = 'CITY = "Taipei"\nRETRIES = 3\nwho = input("Name: ")\nprint(CITY, RETRIES, who)\n'


def _envdefault(name: str) -> ParamDecl:
    return ParamDecl(name=name, binding="envdefault", delivery="env", type="str")


# ---------------------------------------------------------------- drift_lines copy


def test_drift_lines_envdefault_loud_line_is_verbatim():
    # The dedicated LOUD line for an envdefault whose ${NAME:-default} vanished: correctness
    # landmine #1 (the user's env value would be silently ignored). Pin the whole line, prefix
    # included, so any edit to the two-space indent or the wording is caught.
    report = shell.reconcile("echo hello\n", [_envdefault("PORT")])
    lines = analysis.drift_lines(report, "deploy")
    expected = (
        "  PORT is no longer read from the environment (its ${...:-default} was removed or "
        "overridden by a plain assignment) — your value would be silently ignored. "
        "Re-add or resync."
    )
    assert expected in lines


def test_drift_lines_rebind_line_is_verbatim():
    # The positional-fallback warning for an input whose prompt no longer uniquely resolves.
    text = 'value = input("New label: ")\nprint(value)\n'
    report = pyrec.reconcile(
        text, [ParamDecl(name="input-1", binding="input", order=0, prompt="Old label: ")]
    )
    lines = pyrec.drift_lines(report, "myscript")
    expected = (
        "  input-1: its prompt no longer matches a unique input/read call; falling back to "
        "position (still injected — double-check this lands on the right question, "
        "especially if it's a secret)"
    )
    assert expected in lines


# ---------------------------------------------------------------- render_warning copy


def test_render_warning_resync_skipped_is_verbatim():
    # Two-part message; pin both halves exactly (no %(name)s placeholder, so it stands alone).
    assert pyrec.render_warning("resync-skipped") == (
        "Could not parse the script (syntax error); resync skipped. "
        "Parameter definitions are unchanged."
    )


def test_render_warning_resync_rebound_is_verbatim():
    # Exercises the "resync-rebound" dict key (a mangled key would KeyError), the %(name)s
    # substitution (a mangled placeholder or gettext(None) would raise), and the full wording.
    assert pyrec.render_warning("resync-rebound:input-1") == (
        "input-1: re-anchored to its current position after its prompt stopped matching "
        "uniquely; double-check the prompt/secret assignment is still correct."
    )


def test_render_warning_rebound_key_resolves_for_every_name():
    # A second concrete name proves the key lookup + substitution really run (not a fluke of one
    # value); a broken key would raise KeyError here too.
    msg = pyrec.render_warning("resync-rebound:API_KEY")
    assert msg.startswith("API_KEY: re-anchored to its current position")


# ---------------------------------------------------------------- edit_specs resync default


def test_edit_specs_resync_defaults_off():
    # The `resync` keyword defaults to False: a caller that omits it must NOT have missing
    # definitions pruned. Called on analysis.edit_specs directly (the shims pass resync
    # explicitly, which would mask the default) with GONE absent from the script.
    specs = [ParamDecl(name="GONE", binding="const", type="str")]
    result = analysis.edit_specs(PY_SCRIPT, specs, analyze=py_analyze)
    assert [s.name for s in result.specs] == ["GONE"]  # kept: resync did not run
    assert result.warnings == []  # no "resync-dropped:GONE"


def test_edit_specs_resync_true_does_prune():
    # The paired positive: with resync explicitly on, the same GONE spec IS dropped — confirming
    # the default-off test above pins a real behavioural difference, not a dead parameter.
    specs = [ParamDecl(name="GONE", binding="const", type="str")]
    result = analysis.edit_specs(PY_SCRIPT, specs, resync=True, analyze=py_analyze)
    assert result.specs == []
    assert "resync-dropped:GONE" in result.warnings


# ---------------------------------------------------------------- reconcile env coverage


def test_reconcile_managed_envdefault_not_reported_as_new():
    # A managed envdefault that still matches its candidate must be marked covered, so the same
    # candidate is NOT also surfaced in report.new. If coverage recorded the wrong key, the live
    # PORT candidate would wrongly reappear as an unmanaged "new" find.
    report = shell.reconcile('echo "${PORT:-8080}"\n', [_envdefault("PORT")])
    assert [s.name for s in report.ok] == ["PORT"]
    assert report.new == []


if __name__ == "__main__":  # pragma: no cover
    pytest.main([__file__, "-q"])
