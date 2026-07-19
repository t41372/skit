"""Reconciliation tests: const keyed by name, input keyed by order; missing dropped, changed
warned, new not counted as drift."""

from __future__ import annotations

from skit.langs.python import reconcile
from skit.params import Binding, ParamDecl, ParamType


def spec(
    name: str,
    *,
    binding: Binding = "const",
    type: ParamType = "str",
    order: int = -1,
    prompt: str = "",
) -> ParamDecl:
    return ParamDecl(name=name, binding=binding, type=type, order=order, prompt=prompt)


SCRIPT = 'CITY = "Taipei"\nRETRIES = 3\nwho = input("Your name: ")\nprint(who, CITY, RETRIES)\n'


def test_all_ok_no_drift():
    specs = [spec("CITY"), spec("RETRIES", type="int"), spec("input-1", binding="input", order=0)]
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
    report = reconcile.reconcile(text, [spec("input-1", binding="input", order=0)])
    assert not report.has_drift


def test_input_removed_is_missing():
    text = SCRIPT.replace('who = input("Your name: ")', 'who = "nobody"')
    report = reconcile.reconcile(text, [spec("input-1", binding="input", order=0)])
    assert [s.name for s in report.missing] == ["input-1"]


def test_new_input_call_reported_as_new_only():
    text = SCRIPT + 'more = input("More: ")\nprint(more)\n'
    report = reconcile.reconcile(text, [spec("input-1", binding="input", order=0)])
    assert not report.has_drift  # existing definitions are still present; new is not drift
    assert [c.order for c in report.new if c.binding == "input"] == [1]


# ---------- 3a: input matching prefers the stored prompt over bare position ----------


def test_input_prompt_match_survives_an_earlier_insertion_no_drift():
    # A new input() call was inserted BEFORE the managed one, shifting its bare position from 0 to
    # 1. Pre-3a this was invisible (still "ok" by sheer luck of matching *some* candidate at
    # position 0) but could silently rebind a different value onto the wrong question. With the
    # prompt recorded, the match follows the prompt to its new position and is NOT flagged.
    text = 'extra = input("Extra: ")\n' + SCRIPT
    report = reconcile.reconcile(
        text, [spec("input-1", binding="input", order=0, prompt="Your name: ")]
    )
    assert not report.has_drift
    assert [s.name for s in report.ok] == ["input-1"]
    assert report.rebind == []


def test_input_deleted_earlier_call_flags_rebind_instead_of_silent_ok():
    # Reproduces the reconcile/shim gap directly: three input() calls, one stored per question.
    # Deleting the FIRST input() call shifts input-2 and input-3's bare position down by one each.
    # Pre-3a, position-only matching would silently report every one of them "ok" (some candidate
    # still exists at every stored position) even though position 0 now holds a DIFFERENT question
    # than the one input-1 used to describe -- exactly the silent-rebind risk 3a must catch.
    text = (
        'first = input("First: ")\n'
        'second = input("Second: ")\n'
        'third = input("Third: ")\n'
        "print(first, second, third)\n"
    )
    specs = [
        spec("input-1", binding="input", order=0, prompt="First: "),
        spec("input-2", binding="input", order=1, prompt="Second: "),
        spec("input-3", binding="input", order=2, prompt="Third: "),
    ]
    edited = text.replace('first = input("First: ")\n', "")  # delete the first input() call
    report = reconcile.reconcile(edited, specs)
    # input-1 (First:) is genuinely gone: its own prompt matches nothing, and the position it used
    # to occupy (0) is now legitimately owned by input-2's own exact prompt match -- so it must be
    # reported missing, not silently handed input-2's call site.
    assert [s.name for s in report.missing] == ["input-1"]
    # input-2 (Second:) and input-3 (Third:) still resolve correctly by prompt, at their new
    # positions (0 and 1) -- not flagged, because the prompt uniquely identifies each of them
    # despite the shift. This is the concrete proof the fix does its job: no silent swap is even
    # possible here, since the match never falls back to position at all when the prompt still
    # uniquely resolves.
    assert {s.name for s in report.ok} == {"input-2", "input-3"}
    assert report.rebind == []
    assert {s.name for s in report.usable} == {"input-2", "input-3"}


def test_input_rebind_flagged_when_prompt_can_no_longer_disambiguate():
    # When the prompt genuinely can't resolve the call site any more (renamed prompt, but a call
    # still exists at the old bare position), the match must fall back to position AND be flagged
    # as `rebind` -- still usable (no silent drop), but visibly warned (no silent trust either).
    text = 'value = input("New label: ")\nprint(value)\n'
    report = reconcile.reconcile(
        text, [spec("input-1", binding="input", order=0, prompt="Old label: ")]
    )
    assert report.has_drift
    assert [s.name for s, _ in report.rebind] == ["input-1"]
    assert [s.name for s in report.usable] == ["input-1"]  # still injectable, just warned


def test_drift_lines_mention_rebind():
    text = 'value = input("New label: ")\nprint(value)\n'
    report = reconcile.reconcile(
        text, [spec("input-1", binding="input", order=0, prompt="Old label: ")]
    )
    lines = reconcile.drift_lines(report, "myscript")
    assert any("input-1" in line for line in lines)


def test_resync_reanchors_rebound_input_order_and_prompt():
    # --resync must not just prune/retype: an input whose prompt no longer uniquely resolves should
    # be re-anchored to wherever the fallback landed, so the *next* plain run sees an exact prompt
    # match again instead of re-deriving the same fallback (and the same warning) every time.
    text = 'value = input("New label: ")\nprint(value)\n'
    specs = [spec("input-1", binding="input", order=0, prompt="Old label: ")]
    result = reconcile.edit_specs(text, specs, resync=True)
    assert "resync-rebound:input-1" in result.warnings
    (s,) = result.specs
    assert s.prompt == "New label: "
    assert s.order == 0
    # A plain reconcile (no resync) now matches cleanly by prompt -- no more warning.
    report = reconcile.reconcile(text, result.specs)
    assert not report.has_drift


def test_unselected_candidates_are_new_but_not_drift():
    # Onboarding only selected CITY; RETRIES and input are "new" but must never nag the user on
    # every run.
    report = reconcile.reconcile(SCRIPT, [spec("CITY")])
    assert not report.has_drift
    assert {c.name for c in report.new} == {"RETRIES", "input-1"}


def test_input_duplicate_prompt_surplus_is_missing_not_ok_on_delete():
    # Regression: two stored input specs share the identical literal prompt (a retry pattern, e.g.
    # two `input("Go? ")` calls, both managed). The user deletes one of the two calls, leaving a
    # single current call site with that prompt. Pre-fix, match_calls's exact pass let BOTH
    # stored orders exact-match onto that one surviving call site (ambiguous=False), so reconcile
    # reported both "ok" with no drift warning at all -- and shim would go on to splice two
    # replacements over the same input() callee, corrupting the injected copy. The surplus spec
    # must instead come back "missing" (drift), never silently "ok".
    text = 'first = input("Go? ")\nsecond = input("Go? ")\nprint(first, second)\n'
    specs = [
        spec("input-1", binding="input", order=0, prompt="Go? "),
        spec("input-2", binding="input", order=1, prompt="Go? "),
    ]
    edited = text.replace('first = input("Go? ")\n', "")  # delete the first call
    report = reconcile.reconcile(edited, specs)
    assert report.has_drift
    assert [s.name for s in report.missing] == ["input-2"]
    assert [s.name for s in report.ok] == ["input-1"]
    assert report.rebind == []
    assert [s.name for s in report.usable] == ["input-1"]


def test_input_duplicate_prompt_surplus_is_rebind_not_ok_when_position_edited():
    # Same duplicate-prompt setup, but the call at the loser's bare position (1) still exists --
    # its prompt was just edited to something else. The loser can't win an exact match (its
    # candidate was already claimed), so it falls back to position 1, which now answers a
    # *different* question: that must surface as `rebind` (still usable, but warned), never a
    # silent "ok" and never the winner's call site.
    text = 'first = input("Go? ")\nsecond = input("Go? ")\nprint(first, second)\n'
    specs = [
        spec("input-1", binding="input", order=0, prompt="Go? "),
        spec("input-2", binding="input", order=1, prompt="Go? "),
    ]
    edited = text.replace('second = input("Go? ")', 'second = input("Different: ")')
    report = reconcile.reconcile(edited, specs)
    assert report.has_drift
    assert [s.name for s in report.ok] == ["input-1"]
    assert [s.name for s, _ in report.rebind] == ["input-2"]
    assert report.missing == []
    assert {s.name for s in report.usable} == {"input-1", "input-2"}


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
    specs = [ParamDecl(name="CITY", binding="const", type="str")]
    result = reconcile.edit_specs(text, specs, secret=["GONE"])
    assert any("not-managed" in w for w in result.warnings)


def test_edit_specs_not_managed_in_no_secret_warning():
    text = 'CITY = "Taipei"\n'
    specs = [ParamDecl(name="CITY", binding="const", type="str")]
    result = reconcile.edit_specs(text, specs, no_secret=["GONE"])
    assert any("not-managed" in w for w in result.warnings)


def test_edit_specs_not_managed_in_prompts_warning():
    text = 'CITY = "Taipei"\n'
    specs = [ParamDecl(name="CITY", binding="const", type="str")]
    result = reconcile.edit_specs(text, specs, prompts={"GONE": "Enter city:"})
    assert any("not-managed" in w for w in result.warnings)


# ---------- Resync must not wipe definitions on a transient syntax error ----------


def test_resync_on_unparseable_script_leaves_definitions_untouched():
    # A copy-mode script left mid-edit with a syntax error must not have its entire
    # managed-parameter set dropped by --resync. reconcile() can't distinguish "really
    # gone" from "can't parse right now", so _apply_resync must consult report.syntax_error itself.
    specs = [
        ParamDecl(name="API_KEY", binding="const", type="str", secret=True),
        ParamDecl(name="RETRIES", binding="const", type="int"),
        ParamDecl(name="input-1", binding="input", order=0),
    ]
    broken = "API_KEY = 'x'\nRETRIES = (3\n"  # unclosed paren
    result = reconcile.edit_specs(broken, specs, resync=True)
    assert [s.name for s in result.specs] == ["API_KEY", "RETRIES", "input-1"]
    assert result.specs[0].secret is True  # untouched, not rebuilt from a candidate
    assert result.warnings == ["resync-skipped"]


def test_resync_syntax_error_does_not_also_apply_other_edits_incorrectly():
    # A syntax-error resync combined with --remove: the resync guard must only skip the resync
    # step; the rest of edit_specs (remove/add/tweaks) still runs normally on the untouched specs.
    specs = [
        ParamDecl(name="CITY", binding="const", type="str"),
        ParamDecl(name="Y", binding="const"),
    ]
    broken = "def broken(:\n"
    result = reconcile.edit_specs(broken, specs, resync=True, remove=["Y"])
    assert [s.name for s in result.specs] == ["CITY"]
    assert "resync-skipped" in result.warnings


def test_render_warning_resync_skipped():
    msg = reconcile.render_warning("resync-skipped")
    assert msg
    assert "resync" in msg.lower()


# ---------- edit_specs must not crash on duplicate-named specs ----------


def test_edit_specs_remove_with_duplicate_names_does_not_crash():
    # A duplicate-named const (reachable from older metadata or analyzers emitting two same-named
    # candidates during "all" onboarding) used to make `order.remove(name)` leave a
    # dangling name in `order` after `del by_name[name]`, raising KeyError on the final list-comp.
    text = "X = 1\nX = 2\nY = 5\n"
    specs = [spec("X"), spec("X"), spec("Y")]
    result = reconcile.edit_specs(text, specs, remove=["X"])
    assert [s.name for s in result.specs] == ["Y"]
    assert result.warnings == []


def test_edit_specs_resync_drop_with_duplicate_names_does_not_crash():
    # Same dangling-name crash, reached via --resync instead of --remove.
    specs = [spec("X"), spec("X"), spec("Y")]
    text = "Y = 5\n"  # X genuinely no longer exists
    result = reconcile.edit_specs(text, specs, resync=True)
    assert [s.name for s in result.specs] == ["Y"]
    assert result.warnings == ["resync-dropped:X"]  # exactly one, not one per duplicate


def test_edit_specs_dedups_duplicate_names_even_when_untouched():
    # Duplicate names must never survive edit_specs, even when no operation targets them directly:
    # by_name is keyed by name (already deduped), so order must be derived from it, not from the
    # raw (possibly duplicated) `specs` list.
    text = "X = 1\nX = 2\nY = 5\n"
    specs = [spec("X"), spec("X"), spec("Y")]
    result = reconcile.edit_specs(text, specs, secret=["Y"])
    assert [s.name for s in result.specs] == ["X", "Y"]


def test_no_secret_also_clears_the_env_source():
    from skit.langs.python import reconcile
    from skit.langs.python.metawriter import ParamDecl

    specs = [ParamDecl(name="API", binding="const", type="str", secret=True, env_source="MY_KEY")]
    result = reconcile.edit_specs('API = "x"\nprint(API)\n', specs, no_secret=["API"])
    assert result.specs[0].secret is False
    assert result.specs[0].env_source == ""  # an env source only means anything on a secret
