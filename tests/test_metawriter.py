"""MetaWriter ([tool.skit] plain-text writes): A5 fidelity, round-trip, idempotent replacement."""

from __future__ import annotations

import pytest

from skit import pep723
from skit.langs.python import metawriter
from skit.langs.python.metawriter import ParamSpec

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


def test_write_creates_block_adds_no_line_outside_the_block():
    # Regression: fixing _BLOCK_RE's greedy closer (blank-line-swallowing bug) exposed that this
    # "no block yet" path recurses through pep723.inject_block(), which inserts a blank-line
    # separator before the following code for its own (standalone) readability. write_params()
    # immediately overwrites that same block's body with params in the same call, so the separator
    # must not survive — it would be the only line ever added outside the "# /// … # ///" block,
    # violating the comment-only-edits contract (A5) and the corpus byte-fidelity invariant.
    src = "CITY = 'Taipei'\nprint(CITY)\n"
    out = metawriter.write_params(src, PARAMS[:1])
    added = [ln for ln in out.splitlines(keepends=True) if ln not in src.splitlines(keepends=True)]
    assert all(ln.lstrip().startswith("#") for ln in added), added
    assert out.endswith("# ///\n" + src)  # block directly adjacent to the code, no blank line


def test_write_creates_block_after_shebang_adds_no_line_outside_the_block():
    src = "#!/usr/bin/env python3\nCITY = 'Taipei'\nprint(CITY)\n"
    out = metawriter.write_params(src, PARAMS[:1])
    added = [ln for ln in out.splitlines(keepends=True) if ln not in src.splitlines(keepends=True)]
    assert all(ln.lstrip().startswith("#") for ln in added), added
    assert out.startswith("#!/usr/bin/env python3\n")
    assert out.endswith("# ///\nCITY = 'Taipei'\nprint(CITY)\n")


def test_write_creates_block_preserves_a_pre_existing_leading_blank_line():
    # When the source body already begins with a blank line at the block insertion point,
    # inject_block skips its synthetic separator, so _drop_synthetic_separator must return the
    # base unchanged — the user's own blank line survives, and no line is dropped or doubled.
    src = "\nprint(1)\n"
    out = metawriter.write_params(src, PARAMS[:1])
    assert [p.name for p in metawriter.read_params(out)] == ["API_KEY"]
    # exactly one blank line between the block closer and the code (the original's own), not zero, not two
    assert out.endswith("# ///\n\nprint(1)\n")
    assert "# ///\n\n\nprint(1)\n" not in out
    # nothing outside the comment block except the untouched original body
    added = [ln for ln in out.splitlines(keepends=True) if ln not in src.splitlines(keepends=True)]
    assert all(ln.lstrip().startswith("#") for ln in added), added


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


# --- set_dependencies must survive a hand-edited (not skit-generated) deps array closer ---
#
# The skit-generated form always puts the closing "]" alone on its own line, which is why the bug
# (the `in_deps_array` flag never resetting) went unnoticed: only hand-edited variants trigger it.


@pytest.mark.parametrize(
    "deps_block",
    [
        pytest.param(
            '# dependencies = [\n#     "requests"]\n',
            id="close-on-last-element-line",
        ),
        pytest.param(
            '# dependencies = [\n#     "requests",\n# ]  # pin\n',
            id="trailing-comment-on-closer",
        ),
        pytest.param(
            '# dependencies = [\n#     "a",\n#     "b"]\n',
            id="multi-item-close-on-last",
        ),
        pytest.param(
            '# dependencies = [\n#     "pkg[extra]",\n# ]\n',
            id="extras-bracket-in-requirement-string",
        ),
        pytest.param(
            '# dependencies = [\n#     "requests",  # pin later [\n#     "httpx",\n# ]\n',
            id="comment-with-bracket",
        ),
        pytest.param(
            '# dependencies = [\n#     "foo]bar",\n# ]\n',
            id="string-with-bracket",
        ),
    ],
)
def test_set_dependencies_survives_hand_edited_deps_closer(deps_block: str) -> None:
    src = (
        "# /// script\n"
        + deps_block
        + "#\n"
        + "# [tool.skit]\n"
        + "# schema = 1\n"
        + "#\n"
        + "# [[tool.skit.params]]\n"
        + '# name = "API_KEY"\n'
        + "# ///\n"
    )
    # Sanity: read_params on the untouched hand-edited block already sees the param.
    assert [p.name for p in metawriter.read_params(src)] == ["API_KEY"]
    updated = pep723.set_dependencies(src, ["httpx"])
    meta = pep723.parse_block(updated)
    assert meta is not None
    assert meta["dependencies"] == ["httpx"]
    # The [tool.skit] params block must survive — this is the core bug: the old line-shape
    # assumption (closer must be alone on its line) dropped the entire rest of the block body.
    assert [p.name for p in metawriter.read_params(updated)] == ["API_KEY"]
    compile(updated, "<test>", "exec")


# --- fix-review finding: bracket-depth tracking must ignore brackets inside TOML strings and
# inline comments, not just count them naively over the whole line. The earlier fix (see the
# parametrize block above) replaced the "closer alone on its line" check with a raw
# `line.count("[") - line.count("]")`, which itself desyncs on a `[`/`]` living inside a string
# value or an inline `#` comment — reproducing the very param-loss the fix was meant to prevent.


def test_set_dependencies_reproduces_reported_comment_bracket_finding() -> None:
    """Exact scenario from the fix-review finding: an in-array comment containing an unbalanced
    `[` (`#     "requests",  # pin later [`) must not desync the depth counter and swallow the
    following `# [tool.skit]` params block."""
    src = (
        "# /// script\n"
        "# dependencies = [\n"
        '#     "requests",  # pin later [\n'
        '#     "httpx",\n'
        "# ]\n"
        "#\n"
        "# [tool.skit]\n"
        "# schema = 1\n"
        "#\n"
        "# [[tool.skit.params]]\n"
        '# name = "API_KEY"\n'
        "# ///\n"
    )
    # Sanity: the untouched hand-edited block already parses to the one param.
    assert [p.name for p in metawriter.read_params(src)] == ["API_KEY"]
    updated = pep723.set_dependencies(src, ["rich"])
    meta = pep723.parse_block(updated)
    assert meta is not None
    assert meta["dependencies"] == ["rich"]
    # Before the fix this returned [] — the whole [tool.skit] section was silently dropped.
    assert [p.name for p in metawriter.read_params(updated)] == ["API_KEY"]
    compile(updated, "<test>", "exec")


# --- pep723._structural_bracket_delta: direct branch coverage for string-quoting edge cases not
# otherwise exercised by the set_dependencies-level tests above (escaped quotes in a basic string,
# and literal ('...') strings, which TOML never escapes). ---


def test_structural_bracket_delta_escaped_quote_in_basic_string():
    # A backslash-escaped quote inside a basic ("...") string must not end the string early, so a
    # bracket immediately after it (still inside the string) is not counted.
    assert pep723._structural_bracket_delta('"a\\"]b"') == 0


def test_structural_bracket_delta_literal_string_has_no_escapes():
    # TOML literal ('...') strings never treat backslash as an escape: it is a literal character,
    # and a bracket following it (still inside the string) must not be counted either.
    assert pep723._structural_bracket_delta("'a\\]b'") == 0


# --- _BLOCK_RE's closer must not swallow blank lines that follow the block ---


def test_write_params_preserves_blank_lines_after_block():
    src = "# /// script\n# dependencies = []\n# ///\n\n\nimport requests\n"
    out = metawriter.write_params(src, PARAMS[:1])
    # Exactly the two original blank lines must still separate the block from the following code.
    suffix = out.split("# ///\n", 1)[1]
    assert suffix == "\n\nimport requests\n"


def test_set_dependencies_preserves_blank_lines_after_block():
    src = "# /// script\n# dependencies = []\n# ///\n\n\nimport requests\n"
    out = pep723.set_dependencies(src, ["httpx"])
    suffix = out.split("# ///\n", 1)[1]
    assert suffix == "\n\nimport requests\n"


# --- read_params must be total: malformed-but-valid TOML shapes return [] instead of raising ---


@pytest.mark.parametrize(
    "body",
    [
        pytest.param("# dependencies = []\n# tool = 5\n", id="tool-is-scalar"),
        pytest.param("# dependencies = []\n# [tool]\n# skit = 5\n", id="skit-is-scalar"),
        pytest.param("# dependencies = []\n# [tool.skit]\n# params = 5\n", id="params-is-scalar"),
    ],
)
def test_read_params_tolerates_malformed_container_shapes(body: str) -> None:
    src = "# /// script\n" + body + "# ///\n"
    assert metawriter.read_params(src) == []


def test_read_params_tolerates_non_numeric_order() -> None:
    src = (
        "# /// script\n"
        "# dependencies = []\n"
        "#\n"
        "# [tool.skit]\n"
        "# schema = 1\n"
        "#\n"
        "# [[tool.skit.params]]\n"
        '# name = "X"\n'
        '# order = "abc"\n'
        "# ///\n"
    )
    got = metawriter.read_params(src)
    assert len(got) == 1
    assert got[0].name == "X"
    assert got[0].order == -1  # uncoercible -> falls back rather than raising ValueError


def test_from_dict_coerces_non_numeric_order() -> None:
    assert ParamSpec.from_dict({"name": "X", "order": "abc"}).order == -1
    assert ParamSpec.from_dict({"name": "X", "order": None}).order == -1


def test_from_dict_still_coerces_numeric_string_and_float_order() -> None:
    # The fix must not regress previously-working coercions (only non-numeric values fall back).
    assert ParamSpec.from_dict({"name": "X", "order": "3"}).order == 3
    assert ParamSpec.from_dict({"name": "X", "order": 1.9}).order == 1


def test_write_params_survives_unicode_line_separators_in_prompt():
    """str.splitlines() breaks on U+0085/U+2028/U+2029 as well as newlines; if _toml_str
    leaves one raw, _commentify shreds the comment body and every managed param definition
    is lost on the next read. Escaping them keeps the block whole and round-trips the value."""
    for sep in ("\u0085", "\u2028", "\u2029"):
        spec = ParamSpec(name="CITY", kind="const", type="str", default="x", prompt=f"a{sep}b")
        text = metawriter.write_params("CITY = 'x'\n", [spec])
        back = metawriter.read_params(text)
        assert len(back) == 1, f"block lost for separator {sep!r}"
        assert back[0].prompt == f"a{sep}b"
