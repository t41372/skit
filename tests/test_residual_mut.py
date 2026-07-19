"""Behavioral tests written to close mutation-testing assertion gaps (surviving mutants) found by
a full `uv run mutmut run` across src/skit/{analyzer,metawriter,i18n,reconcile,pep723}.py.

Each test targets a specific unasserted behavior (exact line numbers, exact type strings, loop
short-circuit vs skip semantics, precedence of boolean expressions, etc.) rather than re-testing
what tests/test_*.py already cover.
"""

from __future__ import annotations

import locale

import pytest

from skit import analysis, i18n, pep723
from skit.langs.python import analyzer, metawriter, reconcile
from skit.params import ParamDecl

# ---------------------------------------------------------------------------
# analyzer._const_candidates
# ---------------------------------------------------------------------------


def test_const_candidates_tuple_target_not_a_crash():
    """A single-target assignment whose target isn't a plain Name (e.g. tuple/list unpacking)
    must be skipped, not crash. This distinguishes the `or` in
    `not isinstance(target, ast.Name) or value is None` from an `and`: with `and`, a tuple target
    (not a Name, but with a real non-None RHS) would fall through to `target.id`, an AttributeError
    on ast.Tuple."""
    src = "a, b = 1, 2\nFOLLOWUP = 9\n"
    result = analyzer.analyze(src)
    names = {c.name for c in result.candidates}
    assert names == {"FOLLOWUP"}


def test_const_candidates_skip_is_continue_not_break():
    """A statement that fails the target/value shape check must only be skipped, not abort
    scanning the rest of the body."""
    src = "'a plain expression statement, not an assignment'\nAFTER = 1\n"
    result = analyzer.analyze(src)
    names = {c.name for c in result.candidates}
    assert names == {"AFTER"}


def test_const_candidates_private_skip_is_continue_not_break():
    """A leading-underscore name must only be skipped, not abort scanning the rest of the body."""
    src = "_INTERNAL = 1\nPUBLIC = 2\n"
    result = analyzer.analyze(src)
    names = {c.name for c in result.candidates}
    assert names == {"PUBLIC"}


def test_const_candidates_lineno_recorded():
    src = "\n\nTHIRD_LINE = 42\n"
    result = analyzer.analyze(src)
    (only,) = result.candidates
    assert only.lineno == 3


# ---------------------------------------------------------------------------
# analyzer._is_main_guard
# ---------------------------------------------------------------------------


def test_main_guard_rejects_non_compare_test():
    """`if True:` — the test isn't a Compare at all, so it must be rejected up front without
    ever touching `.ops` (a boolean-operator-precedence bug here would instead crash)."""
    src = "if True:\n    BAR = 1\n"
    result = analyzer.analyze(src)
    assert {c.name for c in result.candidates} == set()


def test_main_guard_rejects_chained_comparison():
    """`if 1 == 2 == 3:` is a chained Compare (two ops) and must not be mistaken for the
    single-op `__name__ == "__main__"` guard."""
    src = "if 1 == 2 == 3:\n    FOO = 1\n"
    result = analyzer.analyze(src)
    assert {c.name for c in result.candidates} == set()


def test_main_guard_requires_both_name_and_main_sides():
    """Comparing two plain string constants where one happens to equal "__main__" must not be
    treated as a main guard: `has_name` is required in addition to `has_main`."""
    src = 'if "x" == "__main__":\n    BAZ = 1\n'
    result = analyzer.analyze(src)
    assert {c.name for c in result.candidates} == set()


def test_main_guard_detects_real_guard():
    src = 'if __name__ == "__main__":\n    QUX = 1\n'
    result = analyzer.analyze(src)
    assert {c.name for c in result.candidates} == {"QUX"}


# ---------------------------------------------------------------------------
# analyzer._input_candidates
# ---------------------------------------------------------------------------


def test_input_candidates_type_and_lineno():
    src = "\nvalue = input('Name: ')\n"
    result = analyzer.analyze(src)
    (only,) = [c for c in result.candidates if c.binding == "input"]
    assert only.type == "str"
    assert only.lineno == 2


# ---------------------------------------------------------------------------
# analyzer._detect_frameworks
# ---------------------------------------------------------------------------


def test_detect_frameworks_import_submodule_splits_on_dot():
    """`import click.core` must be recognized as the `click` framework: splitting on "." (not
    whitespace, and not the empty separator) is what strips the submodule suffix."""
    src = "import click.core\n"
    result = analyzer.analyze(src)
    assert result.frameworks == ["click"]


def test_detect_frameworks_import_from_submodule_splits_on_dot():
    src = "from argparse.something import thing\n"
    result = analyzer.analyze(src)
    assert result.frameworks == ["argparse"]


# ---------------------------------------------------------------------------
# analyzer.analyze
# ---------------------------------------------------------------------------


def test_analyze_dedupes_main_guard_candidates_by_actual_name():
    """Two separate `if __name__ == "__main__":` blocks that each define the same name must
    only contribute one candidate: the dedup `seen` set must be keyed by the real candidate name,
    not some other value."""
    src = 'if __name__ == "__main__":\n    FOO = 1\nif __name__ == "__main__":\n    FOO = 2\n'
    result = analyzer.analyze(src)
    assert sum(1 for c in result.candidates if c.name == "FOO") == 1


# ---------------------------------------------------------------------------
# metawriter._toml_str
# ---------------------------------------------------------------------------


def test_toml_str_escapes_newline_return_tab():
    assert metawriter._toml_str("a\nb\rc\td") == r'"a\nb\rc\td"'


def test_toml_str_leaves_plain_space_unescaped():
    """Space (U+0020) is the boundary of the forbidden control-character range and must stay
    literal, not be escaped as \\u0020."""
    assert metawriter._toml_str("a b") == '"a b"'


def test_toml_str_escapes_delete_control_char():
    assert metawriter._toml_str("a\x7fb") == '"a\\u007Fb"'


# ---------------------------------------------------------------------------
# metawriter._commentify
# ---------------------------------------------------------------------------


def test_commentify_strips_trailing_not_leading_whitespace():
    """A blank TOML line becomes a bare "#" (rstrip), not "# " with a hanging trailing space; this
    is exactly what _strip_skit_section's trailing-blank-line check depends on."""
    out = metawriter._commentify("key = 1 \n\nother = 2")
    assert out == ["# key = 1", "#", "# other = 2"]


# ---------------------------------------------------------------------------
# metawriter._strip_skit_section
# ---------------------------------------------------------------------------


def test_strip_skit_section_detects_header_without_space_after_hash():
    """A hand-edited comment block might have "#[tool.skit]" with no space after the hash; the
    section must still be recognized and stripped."""
    body_lines = [
        "# dependencies = []",
        "#",
        "#[tool.skit]",
        "# schema = 1",
        "# [other]",
        "# kept = true",
    ]
    out = metawriter._strip_skit_section(body_lines)
    assert out == ["# dependencies = []", "#", "# [other]", "# kept = true"]


def test_strip_skit_section_drops_trailing_bare_hash_line():
    body_lines = ["# dependencies = []", "#", "# [tool.skit]", "# schema = 1", "#"]
    out = metawriter._strip_skit_section(body_lines)
    assert out == ["# dependencies = []"]


def test_strip_skit_section_drops_trailing_empty_line():
    body_lines = ["# dependencies = []", "", "# [tool.skit]", "# schema = 1", ""]
    out = metawriter._strip_skit_section(body_lines)
    assert out == ["# dependencies = []"]


# ---------------------------------------------------------------------------
# metawriter.write_params
# ---------------------------------------------------------------------------


def test_write_params_raises_with_exact_message_if_inject_block_invariant_broken(monkeypatch):
    """Defensive branch: if pep723.inject_block ever failed to actually create a block, write_params
    must raise with this exact diagnostic message rather than silently mis-writing."""
    monkeypatch.setattr(
        pep723, "inject_block", lambda text, deps, requires_python="", leader="#": text
    )
    with pytest.raises(
        RuntimeError, match=r"^inject_block failed to create an inline-metadata block$"
    ):
        metawriter.write_params("no block here\n", [ParamDecl(name="X", binding="const")])


# ---------------------------------------------------------------------------
# metawriter.read_params
# ---------------------------------------------------------------------------


def test_read_params_skips_non_dict_param_entries_without_crashing():
    text = (
        "# /// script\n"
        "# dependencies = []\n"
        "#\n"
        "# [tool.skit]\n"
        "# schema = 1\n"
        "# params = [1, 2]\n"
        "# ///\n"
    )
    assert metawriter.read_params(text) == []


# ---------------------------------------------------------------------------
# i18n.available_locales
# ---------------------------------------------------------------------------


def test_available_locales_requires_exact_lc_messages_casing(monkeypatch, tmp_path):
    locales_dir = tmp_path / "locales"
    (locales_dir / "xx" / "LC_MESSAGES").mkdir(parents=True)
    (locales_dir / "xx" / "LC_MESSAGES" / "skit.mo").write_bytes(b"")
    monkeypatch.setattr(i18n, "_LOCALES_DIR", locales_dir)
    assert "xx" in i18n.available_locales()


# ---------------------------------------------------------------------------
# i18n._normalize
# ---------------------------------------------------------------------------


def test_normalize_splits_on_first_at_not_last():
    """A hypothetical tag with two "@" segments must keep only the part before the *first* one
    (str.split, not str.rsplit)."""
    assert i18n._normalize("de-DE@euro@extra") == "de-DE"


def test_normalize_splits_on_first_dot_not_last():
    assert i18n._normalize("zh_TW.UTF-8.extra") == "zh-TW"


def test_normalize_actually_strips_at_modifier():
    """The literal separator must really be "@", not some string that can never match."""
    assert i18n._normalize("de-DE@euro") == "de-DE"


def test_normalize_lowercases_long_variant_subtags():
    """A variant subtag of length other than 2 or 4 (e.g. a 5-char POSIX-style variant) is
    lowercased, not uppercased."""
    assert i18n._normalize("en-POSIX") == "en-posix"


# ---------------------------------------------------------------------------
# i18n._expand_chain
# ---------------------------------------------------------------------------


def test_expand_chain_no_alias_does_not_insert_none():
    result = i18n._expand_chain("en")
    assert result == ["en"]
    assert None not in result


# ---------------------------------------------------------------------------
# i18n._config_language
# ---------------------------------------------------------------------------


def test_config_language_reads_back_written_value(monkeypatch, tmp_path):
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
    (tmp_path / "config.toml").write_text('language = "fr-FR"\n', encoding="utf-8")
    assert i18n._config_language() == "fr-FR"


# ---------------------------------------------------------------------------
# i18n.detect_locale
# ---------------------------------------------------------------------------


def test_detect_locale_reads_lc_all_by_exact_env_name(monkeypatch):
    monkeypatch.delenv("SKIT_LANG", raising=False)
    monkeypatch.setenv("LC_ALL", "fr_FR.UTF-8")
    monkeypatch.delenv("LC_MESSAGES", raising=False)
    monkeypatch.delenv("LANG", raising=False)
    assert i18n.detect_locale() == "fr-FR"


def test_detect_locale_reads_lc_messages_by_exact_env_name(monkeypatch):
    monkeypatch.delenv("SKIT_LANG", raising=False)
    monkeypatch.delenv("LC_ALL", raising=False)
    monkeypatch.setenv("LC_MESSAGES", "de_DE.UTF-8")
    monkeypatch.delenv("LANG", raising=False)
    assert i18n.detect_locale() == "de-DE"


def test_detect_locale_c_locale_is_ignored_precisely(monkeypatch):
    """ "C" (the POSIX default) must be treated as no preference, not normalized into a real tag."""
    monkeypatch.delenv("SKIT_LANG", raising=False)
    monkeypatch.setenv("LC_ALL", "C")
    monkeypatch.delenv("LC_MESSAGES", raising=False)
    monkeypatch.setenv("LANG", "C")
    monkeypatch.setattr(locale, "getlocale", lambda: (None, None))
    assert i18n.detect_locale() == ""


def test_detect_locale_system_fallback_uses_language_not_encoding(monkeypatch):
    for var in ("SKIT_LANG", "LC_ALL", "LC_MESSAGES", "LANG"):
        monkeypatch.delenv(var, raising=False)
    monkeypatch.setattr(locale, "getlocale", lambda: ("de_DE", "UTF-8"))
    assert i18n.detect_locale() == "de-DE"


# ---------------------------------------------------------------------------
# i18n._pseudoize
# ---------------------------------------------------------------------------


def test_pseudoize_transforms_full_text_including_first_char():
    assert i18n._pseudoize("Ab %(x)s") == "⟦Àb %(x)s~~⟧"


# ---------------------------------------------------------------------------
# i18n.gettext / i18n.ngettext
# ---------------------------------------------------------------------------


def test_gettext_raises_exact_message_if_translations_stay_unset(monkeypatch):
    monkeypatch.setattr(i18n, "init", lambda *a, **k: None)
    i18n._translations = None
    with pytest.raises(RuntimeError, match=r"^i18n init failed$"):
        i18n.gettext("x")


def test_ngettext_raises_exact_message_if_translations_stay_unset(monkeypatch):
    monkeypatch.setattr(i18n, "init", lambda *a, **k: None)
    i18n._translations = None
    with pytest.raises(RuntimeError, match=r"^i18n init failed$"):
        i18n.ngettext("x", "xs", 2)


def test_ngettext_pseudoizes_when_pseudo_active():
    i18n.init("x-pseudo")
    try:
        text = i18n.ngettext("one item", "%(n)d items", 1)
        assert text.startswith("⟦")
        assert text.endswith("~~⟧")
    finally:
        i18n.init("en")


# ---------------------------------------------------------------------------
# i18n.set_language
# ---------------------------------------------------------------------------


def test_set_language_uses_the_given_tag_not_autodetect(monkeypatch, tmp_path):
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
    monkeypatch.setenv("SKIT_LANG", "de-DE")  # a different locale than the one we set
    try:
        effective = i18n.set_language("zh-TW")
        assert effective == "zh-TW"
    finally:
        i18n.init("en")


# ---------------------------------------------------------------------------
# reconcile.drift_lines
# ---------------------------------------------------------------------------


def test_drift_lines_exact_messages():
    i18n.init("en")
    report = reconcile.Report(
        missing=[ParamDecl(name="GONE", binding="const")],
        changed=[
            (
                ParamDecl(name="RETRIES", binding="const", type="int"),
                analysis.Candidate(binding="const", name="RETRIES", type="str"),
            )
        ],
    )
    lines = reconcile.drift_lines(report, "myscript")
    assert lines == [
        "The parameter definitions for myscript have drifted from the script:",
        "  GONE: injection target no longer exists (dropped from this run's form)",
        "  RETRIES: type changed from int to str in the source"
        " (still injected — double-check the value)",
        # The remedy interpolates the real entry name, not a literal "NAME".
        "To refresh the definitions, run: skit params myscript --resync",
    ]


def test_drift_lines_rebind_uses_input_read_wording():
    """The rebind (positional-fallback) line speaks of an 'input/read call', not an
    'input() call' — the wording is honest across every analyzable kind, not just python's
    input()."""
    i18n.init("en")
    report = reconcile.Report(
        rebind=[
            (
                ParamDecl(name="PW", binding="input", type="str"),
                analysis.Candidate(binding="input", name="PW", type="str"),
            )
        ]
    )
    lines = reconcile.drift_lines(report, "myscript")
    assert any("no longer matches a unique input/read call" in line for line in lines)
    assert lines[-1] == "To refresh the definitions, run: skit params myscript --resync"


# ---------------------------------------------------------------------------
# reconcile.render_warning
# ---------------------------------------------------------------------------


def test_render_warning_exact_messages_for_each_code():
    i18n.init("en")
    assert reconcile.render_warning("not-managed:X") == "X isn't a managed parameter; skipped."
    assert (
        reconcile.render_warning("resync-dropped:X")
        == "Dropped X: it no longer exists in the script."
    )
    assert reconcile.render_warning("already-managed:X") == "X is already managed; skipped."
    assert (
        reconcile.render_warning("not-a-candidate:X")
        == "X isn't a detectable parameter in the current script; skipped."
    )


def test_render_warning_partitions_on_first_colon_only():
    i18n.init("en")
    assert (
        reconcile.render_warning("not-managed:foo:bar")
        == "foo:bar isn't a managed parameter; skipped."
    )


# ---------------------------------------------------------------------------
# reconcile._spec_from_candidate
# ---------------------------------------------------------------------------


def test_spec_from_candidate_preserves_prompt_and_secret():
    cand = analysis.Candidate(
        binding="input", name="input-1", type="str", prompt="Name: ", order=0, secret=True
    )
    from skit.params import ParamDecl

    spec = ParamDecl.from_candidate(cand)
    assert spec.prompt == "Name: "
    assert spec.secret is True


# ---------------------------------------------------------------------------
# reconcile.edit_specs
# ---------------------------------------------------------------------------


def test_edit_specs_resync_defaults_to_off():
    """Without resync=True, a spec whose target no longer exists in the script must be left
    alone (not silently dropped)."""
    text = "CITY = 'Taipei'\n"
    specs = [ParamDecl(name="GONE", binding="const", type="str")]
    result = reconcile.edit_specs(text, specs)
    assert [s.name for s in result.specs] == ["GONE"]
    assert result.warnings == []


# ---------------------------------------------------------------------------
# reconcile.reconcile
# ---------------------------------------------------------------------------


def test_reconcile_processes_specs_after_an_input_spec():
    """An input-kind spec must only `continue` to the next spec, not abort the whole loop —
    a const spec listed after it must still be reconciled."""
    text = "CITY = 'Taipei'\nwho = input('Name: ')\n"
    specs = [
        ParamDecl(name="input-1", binding="input", order=0),
        ParamDecl(name="CITY", binding="const", type="str"),
    ]
    report = reconcile.reconcile(text, specs)
    assert [s.name for s in report.ok] == ["input-1", "CITY"]


def test_reconcile_changed_candidate_not_double_counted_as_new():
    """A const whose type changed is covered (via report.changed); it must not also show up in
    report.new as if it were an undetected/uncovered candidate."""
    text = 'RETRIES = "3"\n'
    specs = [ParamDecl(name="RETRIES", binding="const", type="int")]
    report = reconcile.reconcile(text, specs)
    assert len(report.changed) == 1
    assert report.new == []


# ---------------------------------------------------------------------------
# pep723.suggest_dependencies
# ---------------------------------------------------------------------------


def test_suggest_dependencies_import_submodule_splits_on_dot():
    assert pep723.suggest_dependencies("import click.core\n") == ["click"]


def test_suggest_dependencies_excludes_underscore_prefixed_names():
    assert pep723.suggest_dependencies("import _privatelib\n") == []


# ---------------------------------------------------------------------------
# pep723.set_dependencies
# ---------------------------------------------------------------------------


_BLOCK_WITH_DEPS_AND_PYVER = (
    "# /// script\n"
    '# requires-python = ">=3.10"\n'
    "# dependencies = [\n"
    '#     "old-dep",\n'
    "# ]\n"
    "# ///\n"
    "print('hi')\n"
)


def test_set_dependencies_default_requires_python_omits_the_line():
    out = pep723.set_dependencies("print('hi')\n", ["requests"])
    assert "requires-python" not in out


def test_set_dependencies_replaces_old_requires_python_line():
    out = pep723.set_dependencies(_BLOCK_WITH_DEPS_AND_PYVER, ["new-dep"], ">=3.12")
    assert out.count("requires-python") == 1
    assert ">=3.10" not in out
    assert ">=3.12" in out
    assert "old-dep" not in out


# ---------------------------------------------------------------------------
# pep723.inject_block
# ---------------------------------------------------------------------------


def test_inject_block_default_requires_python_omits_the_line():
    out = pep723.inject_block("print('hi')\n", [])
    assert "requires-python" not in out


def test_inject_block_shebang_only_no_body_no_index_error():
    """A script that is nothing but a shebang line (no body at all) must not crash — insert_at
    equals len(lines), and the follow-up coding-line check must not index past the end."""
    out = pep723.inject_block("#!/usr/bin/env python3\n", [])
    assert out == "#!/usr/bin/env python3\n" + pep723.build_block([])


def test_inject_block_adds_blank_line_before_body():
    out = pep723.inject_block("import requests\n", [])
    assert "# ///\n\nimport requests\n" in out


def test_inject_block_no_double_blank_line_when_body_already_starts_blank():
    out = pep723.inject_block("\nprint('hi')\n", [])
    assert out == pep723.build_block([]) + "\nprint('hi')\n"
