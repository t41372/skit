"""split_requirements: comma-splitting that respects PEP 508 internals.

A requirement string may itself contain commas — in a version-specifier list
(requests>=2,<3), an extras bracket (pkg[security,socks]), a parenthesized specifier
(foo (>=1.0,<2.0)), or a quoted marker value. The CLI previously fed `add --deps`,
the interactive deps prompt, and `deps --set` through a naive split(","), which
shredded "requests>=2,<3" into the two bogus items "requests>=2" and "<3". These
tests pin the splitter's branches and the three CLI call sites.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from skit import cli, pep723, store

runner = CliRunner()


@pytest.fixture
def tty(monkeypatch):
    monkeypatch.setattr("sys.stdin.isatty", lambda: True, raising=False)
    monkeypatch.setattr("sys.stdout.isatty", lambda: True, raising=False)
    monkeypatch.setattr(cli, "_is_interactive", lambda: True)


def _py(tmp_path: Path, body: str) -> Path:
    p = tmp_path / "s.py"
    p.write_text(body, encoding="utf-8")
    return p


# --------------------------------------------------------------------------
# unit: split_requirements
# --------------------------------------------------------------------------


# ---------------------------------------------------------------- comment-leader generalization


def test_block_re_hash_pattern_is_byte_identical_to_the_frozen_literal():
    """The '#'-leader block regex MUST equal the exact historical frozen pattern: the whole Python
    golden corpus and every existing user file depend on it byte-for-byte. Generalizing the engine to
    also serve '//' (JS/TS) must not perturb the '#' path by a single character."""
    frozen = r"(?m)^# /// script\s*$\n(?P<body>(?:^#(?:| .*)$\n)*?)^# ///[^\S\n]*$\n?"
    assert pep723._block_re("#").pattern == frozen


def test_block_re_double_slash_pattern_mirrors_the_hash_form():
    assert pep723._block_re("//").pattern == (
        r"(?m)^// /// script\s*$\n(?P<body>(?:^//(?:| .*)$\n)*?)^// ///[^\S\n]*$\n?"
    )


def test_slash_block_round_trips_with_shebang_skip():
    """A `// /// script` block round-trips on a shebang'd file, and inject_block skips the `#!` line
    exactly as it does for the '#' leader (a `#!/usr/bin/env node` shebang is legal in .mjs)."""
    src = "#!/usr/bin/env node\nconst X = 5;\n"
    out = pep723.inject_block(src, [], leader="//")
    assert pep723.has_block(out, "//")
    assert out.startswith("#!/usr/bin/env node\n")
    assert out.index("#!") < out.index("// /// script")
    assert pep723.parse_block(out, "//") == {"dependencies": []}


def test_simple_list_splits():
    assert pep723.split_requirements("requests, rich") == ["requests", "rich"]


def test_single_item_no_commas():
    assert pep723.split_requirements("requests") == ["requests"]


def test_specifier_commas_stay_joined():
    assert pep723.split_requirements("requests>=2,<3") == ["requests>=2,<3"]


def test_specifier_lists_split_only_between_requirements():
    assert pep723.split_requirements("requests>=2,<3, pillow!=9.0,>=8") == [
        "requests>=2,<3",
        "pillow!=9.0,>=8",
    ]


def test_spaces_around_specifier_commas():
    # The continuation clause may be padded with spaces; the comma still belongs
    # to the specifier because what follows is an operator, not a name.
    assert pep723.split_requirements("foo >= 1 , < 2 , bar") == ["foo >= 1 , < 2", "bar"]


def test_extras_bracket_commas_stay_joined():
    assert pep723.split_requirements("requests[security,socks]>=2, rich") == [
        "requests[security,socks]>=2",
        "rich",
    ]


def test_parenthesized_specifier_commas_stay_joined():
    assert pep723.split_requirements("foo (>=1.0,<2.0), bar") == ["foo (>=1.0,<2.0)", "bar"]


def test_double_quoted_marker_comma_stays_joined():
    assert pep723.split_requirements('a; sys_platform in "linux,darwin", b') == [
        'a; sys_platform in "linux,darwin"',
        "b",
    ]


def test_single_quoted_marker_comma_stays_joined():
    assert pep723.split_requirements("a; extra in 'x,y', b") == ["a; extra in 'x,y'", "b"]


def test_name_starting_with_digit_splits():
    # PEP 508 names may start with a digit; isalnum (not isalpha) is the predicate.
    assert pep723.split_requirements("rich, 2captcha-python") == ["rich", "2captcha-python"]


def test_trailing_comma_dropped():
    assert pep723.split_requirements("requests>=2,<3,") == ["requests>=2,<3"]


def test_empty_and_blank_input():
    assert pep723.split_requirements("") == []
    assert pep723.split_requirements("   ") == []


def test_uppercase_x_in_name_is_ordinary_text():
    # Guards the bracket character classes against corruption: an 'X' in a package
    # name must not perturb the bracket-nesting depth (kills the "XX([XX" mutants).
    assert pep723.split_requirements("pkgX, rich") == ["pkgX", "rich"]


def test_nested_brackets_tracked_by_depth_not_flag():
    # Depth must accumulate (+=), not be pinned to 1: with nesting, a pinned depth
    # hits zero at the first closer and lets an inner comma split mid-requirement.
    assert pep723.split_requirements("a[[x],y], b") == ["a[[x],y]", "b"]


def test_next_nonspace_end_of_text_is_empty_string():
    # The trailing-comma path relies on the exact "" sentinel; through the caller a
    # non-empty alnum return is coincidentally equivalent, so pin the contract here.
    assert pep723._next_nonspace("a,  ", 2) == ""
    assert pep723._next_nonspace("a, b", 2) == "b"


# --------------------------------------------------------------------------
# CLI call sites
# --------------------------------------------------------------------------


def test_add_dep_flags_carry_specifier_commas(tmp_path):
    p = _py(tmp_path, "import requests\nprint(requests)\n")
    result = runner.invoke(
        cli.app,
        ["add", str(p), "--name", "r", "--dep", "requests>=2,<3", "--dep", "rich", "--no-input"],
    )
    assert result.exit_code == 0, result.output
    entry = store.resolve("r")
    block = pep723.parse_block(entry.script_path.read_text(encoding="utf-8"))
    assert block is not None
    assert block["dependencies"] == ["requests>=2,<3", "rich"]


def test_interactive_deps_answer_keeps_specifier_commas(monkeypatch, tty):
    answers = iter(["requests>=2,<3, rich", ""])
    monkeypatch.setattr(cli.Prompt, "ask", lambda *a, **k: next(answers))
    deps, py = cli._resolve_python_metadata(
        "import requests\nprint(requests)\n", None, None, no_input=False
    )
    assert deps == ["requests>=2,<3", "rich"]
    assert py == ""


def test_deps_dep_flags_carry_specifier_commas(tmp_path):
    store.add_python(_py(tmp_path, "print(1)\n"), name="a")
    result = runner.invoke(cli.app, ["deps", "a", "--dep", "requests>=2,<3", "--dep", "rich"])
    assert result.exit_code == 0, result.output
    assert store.resolve("a").meta.dependencies == ["requests>=2,<3", "rich"]


# ---------------------------------------------------------------------------
# build_block / set_dependencies: TOML-string escaping of dependency values
# ---------------------------------------------------------------------------


def test_build_block_escapes_double_quoted_marker():
    """A PEP 508 marker carries embedded double quotes (python_version >= "3.8"). Emitted
    raw into a "..." TOML string it would terminate the string early, so the generated
    block fails to re-parse and the whole dependency list is silently lost. The block must
    round-trip through parse_block intact."""
    dep = 'requests; python_version >= "3.8"'
    block = pep723.build_block([dep])
    meta = pep723.parse_block(block)
    assert meta is not None, "generated block does not parse"
    assert meta["dependencies"] == [dep]


def test_set_dependencies_escapes_double_quoted_marker():
    text = "# /// script\n# dependencies = []\n# ///\nprint(1)\n"
    dep = 'httpx; sys_platform == "darwin"'
    out = pep723.set_dependencies(text, [dep])
    meta = pep723.parse_block(out)
    assert meta is not None
    assert meta["dependencies"] == [dep]


def test_build_block_escapes_backslash_in_dependency():
    dep = 'pkg; platform_release == "5.10\\test"'
    meta = pep723.parse_block(pep723.build_block([dep]))
    assert meta is not None
    assert meta["dependencies"] == [dep]
