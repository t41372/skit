"""The ``path`` parameter type (docs/design/path.md, P1a): str semantics on every value
surface, both serialization homes carry it, reconcile treats a declared path over a
source-derived str as a refinement (never drift), resync preserves it, and unknown
types keep degrading to str — the same mechanism an older skit's read relies on.
Analyzer detection is pinned in test_argspec.py / test_argspec_click_typer.py."""

from __future__ import annotations

from skit import flows, params, tui_form
from skit.langs.python import reconcile
from skit.params import ParamDecl


def _decl(name: str = "SRC", *, prompt: str = "", secret: bool = False) -> ParamDecl:
    return ParamDecl(name=name, binding="const", type="path", prompt=prompt, secret=secret)


# ---------- the type axis ----------


def test_path_is_an_allowed_type():
    assert params.as_param_type("path") == "path"
    assert "path" in params.ALLOWED_TYPES


def test_unknown_type_still_degrades_to_str():
    # The graceful-degrade mechanism an older skit's read of type="path" relies on:
    # anything outside the closed set coerces to str, in both serialization homes.
    assert (
        ParamDecl.from_block_dict({"name": "X", "kind": "const", "type": "pathlike"}).type == "str"
    )
    assert ParamDecl.from_meta_dict({"name": "X", "type": "pathlike"}).type == "str"


def test_block_round_trip_carries_path():
    d = _decl()
    assert ParamDecl.from_block_dict(d.to_block_dict()).type == "path"


def test_meta_round_trip_carries_path():
    d = ParamDecl(name="src", delivery="flag", type="path")
    assert ParamDecl.from_meta_dict(d.to_meta_dict()).type == "path"


def test_coerce_default_path_keeps_raw_string():
    # path carries str semantics: no coercion, no existence check.
    assert params.coerce_default("./no such file.csv", "path") == "./no such file.csv"


def test_edit_declared_accepts_path_type():
    decls = [ParamDecl(name="src", delivery="flag")]
    res = params.edit_declared(decls, types={"src": "path"})
    assert res.decls[0].type == "path"
    assert res.warnings == []


# ---------- reconcile: refinement, not drift ----------

SCRIPT = 'SRC = "./data.csv"\nRETRIES = 3\nprint(SRC, RETRIES)\n'


def test_reconcile_path_over_str_const_is_refinement():
    report = reconcile.reconcile(SCRIPT, [_decl()])
    assert not report.has_drift
    assert report.changed == []
    assert [s.name for s in report.usable] == ["SRC"]


def test_reconcile_path_over_int_const_is_drift():
    report = reconcile.reconcile(SCRIPT, [_decl(name="RETRIES")])
    assert report.has_drift
    assert [(s.name, c.type) for s, c in report.changed] == [("RETRIES", "int")]


def test_resync_preserves_declared_path():
    res = reconcile.edit_specs(SCRIPT, [_decl(secret=False, prompt="Which file? ")], resync=True)
    s = res.specs[0]
    assert s.type == "path"  # the refinement survives --resync
    assert s.prompt == "Which file? "
    assert "resync-dropped:SRC" not in res.warnings


def test_resync_still_corrects_real_type_drift():
    # The refinement rule is path-over-str ONLY: a path declared over an int constant
    # is real drift and resync re-anchors it to the source truth.
    res = reconcile.edit_specs(SCRIPT, [_decl(name="RETRIES")], resync=True)
    assert res.specs[0].type == "int"


# ---------- form projection and validation ----------


def test_formfield_carries_path_for_every_delivery():
    kinds = {
        delivery: flows.FormField.from_decl(
            ParamDecl(name="src", binding=binding, delivery=delivery, type="path")
        ).kind
        for delivery, binding in (
            ("inject", "const"),
            ("flag", "none"),
            ("env", "envdefault"),
            ("placeholder", "none"),
        )
    }
    assert kinds == {"inject": "path", "flag": "path", "env": "path", "placeholder": "path"}


def test_degraded_flag_field_still_renders_free_text():
    d = ParamDecl(name="src", delivery="flag", type="path", degraded=True)
    assert flows.FormField.from_decl(d).kind == "str"


def test_validate_value_path_is_free_text():
    f = flows.FormField(key="src", label="src", kind="path")
    assert flows.validate_value(f, "./definitely/not/created/yet.csv") is None


def test_type_label_path():
    assert tui_form._type_label("path") == "path"
