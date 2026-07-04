"""MetaWriter ([tool.skit] plain-text writes): A5 fidelity, round-trip, idempotent replacement."""

from __future__ import annotations

from skit import metawriter, pep723
from skit.metawriter import ParamSpec

PARAMS = [
    ParamSpec(name="API_KEY", kind="const", type="str", default="abc", secret=True),
    ParamSpec(name="RETRIES", kind="const", type="int", default=3),
    ParamSpec(name="input-1", kind="input", type="str", prompt="City: ", order=0),
]


def test_write_creates_block_when_missing():
    src = "print('hi')\n"
    out = metawriter.write_params(src, PARAMS)
    assert pep723.has_block(out)
    assert "print('hi')" in out  # user code is untouched
    got = metawriter.read_params(out)
    assert [p.name for p in got] == ["API_KEY", "RETRIES", "input-1"]


def test_roundtrip_types_and_fields():
    out = metawriter.write_params("x = 1\n", PARAMS)
    got = {p.name: p for p in metawriter.read_params(out)}
    assert got["API_KEY"].secret is True
    assert got["API_KEY"].default == "abc"
    assert got["RETRIES"].default == 3
    assert got["RETRIES"].type == "int"
    assert got["input-1"].order == 0
    assert got["input-1"].prompt == "City: "


def test_preserves_existing_dependencies():
    src = (
        "# /// script\n"
        '# requires-python = ">=3.11"\n'
        "# dependencies = [\n"
        '#     "requests",\n'
        "# ]\n"
        "# ///\n"
        "import requests\n"
    )
    out = metawriter.write_params(src, PARAMS)
    meta = pep723.parse_block(out)
    assert meta is not None
    assert meta["dependencies"] == ["requests"]
    assert meta["requires-python"] == ">=3.11"
    assert len(metawriter.read_params(out)) == 3


def test_rewrite_replaces_not_duplicates():
    out1 = metawriter.write_params("x = 1\n", PARAMS)
    out2 = metawriter.write_params(out1, PARAMS[:1])
    got = metawriter.read_params(out2)
    assert [p.name for p in got] == ["API_KEY"]
    assert out2.count("[tool.skit]") == 1


def test_empty_params_removes_section():
    out1 = metawriter.write_params("x = 1\n", PARAMS)
    out2 = metawriter.write_params(out1, [])
    assert metawriter.read_params(out2) == []
    assert "[tool.skit]" not in out2
    assert pep723.has_block(out2)  # the PEP 723 block itself remains (dependencies preserved)


def test_string_escaping():
    params = [ParamSpec(name="MSG", default='say "hi" \\ bye')]
    out = metawriter.write_params("x = 1\n", params)
    got = metawriter.read_params(out)
    assert got[0].default == 'say "hi" \\ bye'


def test_shebang_preserved_first():
    src = "#!/usr/bin/env python3\nprint('x')\n"
    out = metawriter.write_params(src, PARAMS[:1])
    assert out.startswith("#!/usr/bin/env python3\n")


def test_script_still_valid_python():
    src = "CITY = 'Taipei'\nprint(CITY)\n"
    out = metawriter.write_params(src, PARAMS)
    compile(out, "<test>", "exec")  # injected section is pure comments; semantics unchanged (A5)


def test_set_dependencies_preserves_tool_skit():
    # Updating deps must not destroy [tool.skit] parameter definitions (core constraint of
    # `skit deps`).
    out = metawriter.write_params("x = 1\n", PARAMS)
    updated = pep723.set_dependencies(out, ["requests", "rich"], ">=3.12")
    meta = pep723.parse_block(updated)
    assert meta is not None
    assert meta["dependencies"] == ["requests", "rich"]
    assert meta["requires-python"] == ">=3.12"
    assert [p.name for p in metawriter.read_params(updated)] == ["API_KEY", "RETRIES", "input-1"]
    # Clear deps; params must still be there
    cleared = pep723.set_dependencies(updated, [])
    cleared_meta = pep723.parse_block(cleared)
    assert cleared_meta is not None
    assert cleared_meta["dependencies"] == []
    assert len(metawriter.read_params(cleared)) == 3
    compile(cleared, "<test>", "exec")


def test_set_dependencies_without_block_injects():
    out = pep723.set_dependencies("print('x')\n", ["httpx"])
    meta = pep723.parse_block(out)
    assert meta is not None
    assert meta["dependencies"] == ["httpx"]
