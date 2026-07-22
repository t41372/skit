"""Mutation-kill tests for pep723 internals.

These pin behaviours that the wider suite exercised only indirectly:

* ``_strip_comment_prefix`` — the exact prefix it strips (leader + one space) and its
  default leader, since ``parse_block`` always passes the leader explicitly.
* ``_toml_str`` — the *canonical* escape it emits for the newline-class characters
  (``\\n`` / ``\\r`` / ``\\t``, not the longer ``\\u000A`` form). A newline leaking into
  the single-comment-line block would corrupt it, so the serializer's escape table is a
  real contract.
* ``_structural_bracket_delta`` — bracket-nesting arithmetic that ignores string
  internals: it must count each opener (``+=``, not pinned to 1), each closer once
  (``-= 1``), and keep scanning past a backslash escape inside a ``"`` string.
* ``set_dependencies`` / ``inject_block`` — the comment ``leader`` must thread through to
  the block regex, the strip, and the fallback inject, so a ``//`` (JS/TS) script is not
  silently handled as if it were ``#`` (Python).
"""

from __future__ import annotations

from skit import pep723

# --------------------------------------------------------------------------
# _strip_comment_prefix: prefix shape + default leader
# --------------------------------------------------------------------------


def test_strip_comment_prefix_default_leader_is_hash():
    # The default leader is "#": parse_block always passes a leader, so only the default
    # pins this. A corrupted default ("XX#XX") mis-slices the line.
    assert pep723._strip_comment_prefix("# hello") == "hello"


def test_strip_comment_prefix_strips_leader_and_single_space():
    # The stripped prefix is exactly `leader + " "`; a mangled separator leaves a stray
    # leading space (" hello") instead of "hello".
    assert pep723._strip_comment_prefix("# hello", "#") == "hello"
    assert pep723._strip_comment_prefix("// hello", "//") == "hello"


def test_strip_comment_prefix_bare_leader_yields_empty():
    # A bare leader (no trailing text) strips to "" on both branches.
    assert pep723._strip_comment_prefix("#", "#") == ""


# --------------------------------------------------------------------------
# _toml_str: canonical escapes for newline-class characters
# --------------------------------------------------------------------------


def test_toml_str_escapes_newline_as_backslash_n():
    # A literal newline must serialize to the short two-char backslash-n escape (not the
    # longer \\u000A form, and never a raw newline that would break out of the single-line
    # PEP 723 comment the block is built from).
    assert pep723._toml_str("a\nb") == '"a\\nb"'


def test_toml_str_escapes_carriage_return_as_backslash_r():
    assert pep723._toml_str("a\rb") == '"a\\rb"'


def test_toml_str_escapes_tab_as_backslash_t():
    assert pep723._toml_str("a\tb") == '"a\\tb"'


def test_toml_str_newline_escape_round_trips_in_block():
    # End-to-end: a dependency value carrying a newline survives build -> parse intact,
    # which only holds if the newline is escaped rather than left to split the comment.
    dep = "pkg; marker == 'a\nb'"
    meta = pep723.parse_block(pep723.build_block([dep]))
    assert meta is not None
    assert meta["dependencies"] == [dep]


# --------------------------------------------------------------------------
# _structural_bracket_delta: nesting arithmetic, escape-aware
# --------------------------------------------------------------------------


def test_structural_delta_counts_each_opener():
    # Two openers => +2. A depth pinned to 1 (delta = 1) would report only 1 and desync
    # the closer search on a nested array.
    assert pep723._structural_bracket_delta("[[") == 2


def test_structural_delta_balanced_pair_is_zero():
    # One opener (+1) and one closer (-1) cancel. A closer that subtracted 2 would land at
    # -1 and prematurely end the array scan.
    assert pep723._structural_bracket_delta("[]") == 0
    assert pep723._structural_bracket_delta("]") == -1


def test_structural_delta_keeps_scanning_after_string_escape():
    # A backslash escape inside a "..." string skips the escaped char but must NOT abandon
    # the rest of the line: the structural "[" after the string still counts (+1). Aborting
    # (break) at the escape would miss it and return 0.
    assert pep723._structural_bracket_delta('"x\\qy"[') == 1


def test_structural_delta_ignores_brackets_inside_strings_and_comments():
    # Sanity anchor for the walker: brackets inside a quoted value or after an inline `#`
    # comment are data/prose, not structure.
    assert pep723._structural_bracket_delta('"foo]bar"') == 0
    assert pep723._structural_bracket_delta("] # pin later [") == -1


# --------------------------------------------------------------------------
# set_dependencies / inject_block: the comment leader threads through
# --------------------------------------------------------------------------


def test_set_dependencies_no_block_injects_with_the_given_leader():
    # No existing block => falls back to inject_block, which must receive the "//" leader
    # (not the default "#") so a JS/TS script gets a `//` block it can re-parse.
    out = pep723.set_dependencies("const x = 1;\n", ["left-pad"], leader="//")
    assert pep723.has_block(out, "//")
    assert pep723.parse_block(out, "//") == {"dependencies": ["left-pad"]}


def test_set_dependencies_updates_existing_slash_block():
    # An existing `//` block's dependency line must be recognised (stripped with the "//"
    # leader) and replaced. Stripping with "#" instead leaves the old array in place,
    # producing a duplicate `dependencies` key that fails to parse.
    text = '// /// script\n// dependencies = [\n//     "old",\n// ]\n// ///\nconst x = 1;\n'
    out = pep723.set_dependencies(text, ["new"], leader="//")
    meta = pep723.parse_block(out, "//")
    assert meta is not None
    assert meta["dependencies"] == ["new"]


def test_set_dependencies_keeps_other_sections_after_inline_empty_array():
    # A single-line `dependencies = []` must NOT open the multi-line array state (net == 0
    # is not > 0). If it did, the following `[tool.skit]` line would be swallowed as array
    # content and lost.
    text = '# /// script\n# dependencies = []\n# [tool.skit]\n# name = "x"\n# ///\nprint(1)\n'
    out = pep723.set_dependencies(text, ["requests"])
    assert "[tool.skit]" in out
    meta = pep723.parse_block(out)
    assert meta is not None
    assert meta["dependencies"] == ["requests"]
    assert meta["tool"]["skit"]["name"] == "x"


def test_inject_block_with_existing_slash_block_is_unchanged():
    # inject_block must detect the existing block using the given leader. Checking for a
    # "#" block instead would find none and inject a second, duplicate `//` block.
    text = "// /// script\n// dependencies = []\n// ///\nconst x = 1;\n"
    assert pep723.inject_block(text, ["requests"], leader="//") == text


# --------------------------------------------------------------------------
# _is_local_module: sibling shadows a same-named PyPI distribution
# --------------------------------------------------------------------------


def test_is_local_module_py_file_arm(tmp_path):
    # The `.py` arm ALONE: only a sibling `helpers.py` exists (no `helpers/` dir). The first
    # arm (`(dir / "helpers.py").is_file()`) must return True on its own — a mutant that swaps
    # it to `.is_dir()` sees no directory and returns False. Asserted both directly and through
    # the suggestion filter that consumes it.
    (tmp_path / "helpers.py").write_text("X = 1\n", encoding="utf-8")
    assert pep723._is_local_module(tmp_path, "helpers") is True
    assert pep723.suggest_dependencies("import helpers\n", script_dir=tmp_path) == []


def test_is_local_module_package_arm(tmp_path):
    # The `__init__.py` arm ALONE: only a sibling `helpers/` package exists (no `helpers.py`).
    # A regular package really does shadow a same-named distribution, so this arm must return
    # True on its own. The `or` between the arms is likewise pinned: each test leaves exactly
    # one arm true, so an `and` mutant fails both.
    (tmp_path / "helpers").mkdir()
    (tmp_path / "helpers" / "__init__.py").write_text("X = 1\n", encoding="utf-8")
    assert pep723._is_local_module(tmp_path, "helpers") is True
    assert pep723.suggest_dependencies("import helpers\n", script_dir=tmp_path) == []


def test_is_local_module_namespace_portion_with_modules_arm(tmp_path):
    # The glob arm ALONE: a sibling directory with no `__init__.py` but real modules inside is
    # a namespace portion the script can import from, so it still counts as local.
    (tmp_path / "helpers").mkdir()
    (tmp_path / "helpers" / "thing.py").write_text("X = 1\n", encoding="utf-8")
    assert pep723._is_local_module(tmp_path, "helpers") is True
    assert pep723.suggest_dependencies("import helpers\n", script_dir=tmp_path) == []


def test_data_directory_does_not_suppress_a_real_dependency(tmp_path):
    # The regression this rule exists for: a directory carrying no Python is not an import.
    # PEP 420 makes it a namespace PORTION at most, and the finder keeps scanning sys.path,
    # so the installed `rich` still wins — dropping the dependency on its account produced a
    # stored copy that died with ModuleNotFoundError on its first run.
    (tmp_path / "rich").mkdir()
    (tmp_path / "rich" / "notes.txt").write_text("data\n", encoding="utf-8")
    assert pep723._is_local_module(tmp_path, "rich") is False
    assert pep723.suggest_dependencies("import rich\n", script_dir=tmp_path) == ["rich"]


def test_is_local_module_none_script_dir_is_false():
    # script_dir=None short-circuits to False and never touches the filesystem (a `None / "x"`
    # would raise). A mutant inverting the guard would fall through to the path arithmetic and
    # crash; a `return False` -> `return True` mutant would wrongly call a script-less name local.
    assert pep723._is_local_module(None, "helpers") is False


def test_is_local_module_no_sibling_is_false(tmp_path):
    # An otherwise-empty directory: neither `helpers.py` nor `helpers/` is present, so the name
    # is NOT local and stays a real suggestion (the filter is precise, not blanket).
    assert pep723._is_local_module(tmp_path, "helpers") is False
    assert pep723.suggest_dependencies("import helpers\n", script_dir=tmp_path) == ["helpers"]


# --------------------------------------------------------------------------
# suggest_dependencies: underscore-led (private/C-extension) imports are dropped
# --------------------------------------------------------------------------


def test_suggest_dependencies_drops_underscore_led_import(monkeypatch):
    # `not m.startswith("_")` excludes private/dunder import names (C-extension shims like
    # `_mycext`) from suggestions. The surviving mutant `startswith("XX_XX")` never matches a real
    # module, so the filter goes dead and the underscore name flows through.
    #
    # A leading-underscore name is ALSO rejected downstream by requirement_error (PEP 508 forbids
    # it), which would mask the mutant — with that second filter live, both original and mutant
    # return []. Neutralize requirement_error so THIS filter is the only thing deciding: now the
    # original still drops `_mycext` (→ []) while the mutant would suggest it (→ ["_mycext"]).
    # script_dir stays None, so the local-module filter is not involved.
    monkeypatch.setattr(pep723, "requirement_error", lambda value: None)
    result = pep723.suggest_dependencies("import _mycext\n")
    assert "_mycext" not in result
    assert result == []
