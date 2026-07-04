"""Reconciliation tests: const keyed by name, input keyed by order; missing dropped, changed
warned, new not counted as drift."""

from __future__ import annotations

from skit import reconcile
from skit.metawriter import ParamSpec


def spec(name: str, *, kind: str = "const", type: str = "str", order: int = -1) -> ParamSpec:
    return ParamSpec(name=name, kind=kind, type=type, order=order)


SCRIPT = 'CITY = "Taipei"\nRETRIES = 3\nwho = input("Your name: ")\nprint(who, CITY, RETRIES)\n'


def test_all_ok_no_drift():
    specs = [spec("CITY"), spec("RETRIES", type="int"), spec("input-1", kind="input", order=0)]
    report = reconcile.reconcile(SCRIPT, specs)
    assert not report.has_drift
    assert report.usable == specs
    assert report.new == []


def test_const_missing_by_name():
    report = reconcile.reconcile(SCRIPT, [spec("GONE")])
    assert report.has_drift
    assert [s.name for s in report.missing] == ["GONE"]
    assert report.usable == []


def test_const_renamed_is_missing_plus_new():
    # User renamed CITY to TOWN: old definition is missing, new name appears in new (informational,
    # not considered drift).
    text = SCRIPT.replace("CITY", "TOWN")
    report = reconcile.reconcile(text, [spec("CITY")])
    assert [s.name for s in report.missing] == ["CITY"]
    assert "TOWN" in [c.name for c in report.new]


def test_const_type_changed_still_usable():
    text = SCRIPT.replace("RETRIES = 3", 'RETRIES = "3"')
    report = reconcile.reconcile(text, [spec("RETRIES", type="int")])
    assert report.has_drift
    assert [(s.name, c.type) for s, c in report.changed] == [("RETRIES", "str")]
    assert [s.name for s in report.usable] == ["RETRIES"]  # still injectable, but warned


def test_input_matched_by_order_not_position_in_file():
    # Code was inserted before the script; the input() line number changed but it's still call 0 —
    # not considered drift.
    text = "import os\nprint(os.name)\n" + SCRIPT
    report = reconcile.reconcile(text, [spec("input-1", kind="input", order=0)])
    assert not report.has_drift


def test_input_removed_is_missing():
    text = SCRIPT.replace('who = input("Your name: ")', 'who = "nobody"')
    report = reconcile.reconcile(text, [spec("input-1", kind="input", order=0)])
    assert [s.name for s in report.missing] == ["input-1"]


def test_new_input_call_reported_as_new_only():
    text = SCRIPT + 'more = input("More: ")\nprint(more)\n'
    report = reconcile.reconcile(text, [spec("input-1", kind="input", order=0)])
    assert not report.has_drift  # existing definitions are still present; new is not drift
    assert [c.order for c in report.new if c.kind == "input"] == [1]


def test_unselected_candidates_are_new_but_not_drift():
    # Onboarding only selected CITY; RETRIES and input are "new" but must never nag the user on
    # every run.
    report = reconcile.reconcile(SCRIPT, [spec("CITY")])
    assert not report.has_drift
    assert {c.name for c in report.new} == {"RETRIES", "input-1"}


def test_syntax_error_marks_all_missing():
    report = reconcile.reconcile("def broken(:\n", [spec("CITY")])
    assert report.syntax_error
    assert [s.name for s in report.missing] == ["CITY"]
    assert report.usable == []


def test_drift_lines_mention_old_and_new_type():
    text = SCRIPT.replace("RETRIES = 3", 'RETRIES = "3"')
    report = reconcile.reconcile(text, [spec("RETRIES", type="int"), spec("GONE")])
    lines = reconcile.drift_lines(report, "myscript")
    joined = "\n".join(lines)
    assert "GONE" in joined
    assert "RETRIES" in joined
    assert "int" in joined
    assert "str" in joined


# ---------- edit_specs: not-managed warning branches ----------


def test_edit_specs_not_managed_in_secret_warning():
    """Passing a name that isn't managed into secret= must record a warning, not crash."""
    text = 'CITY = "Taipei"\n'
    specs = [ParamSpec(name="CITY", kind="const", type="str")]
    result = reconcile.edit_specs(text, specs, secret=["GONE"])
    assert any("not-managed" in w for w in result.warnings)


def test_edit_specs_not_managed_in_no_secret_warning():
    text = 'CITY = "Taipei"\n'
    specs = [ParamSpec(name="CITY", kind="const", type="str")]
    result = reconcile.edit_specs(text, specs, no_secret=["GONE"])
    assert any("not-managed" in w for w in result.warnings)


def test_edit_specs_not_managed_in_prompts_warning():
    text = 'CITY = "Taipei"\n'
    specs = [ParamSpec(name="CITY", kind="const", type="str")]
    result = reconcile.edit_specs(text, specs, prompts={"GONE": "Enter city:"})
    assert any("not-managed" in w for w in result.warnings)
