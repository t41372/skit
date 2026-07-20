"""Residual coverage + adversarial probing for i18n, metawriter, pep723, reconcile.

Targets small remaining gaps (see module docstrings for the exact lines) plus behaviors that
were never exercised regardless of coverage: locale fallback edge cases, malformed PEP 723
blocks, metawriter round-trip fidelity (CRLF/unicode/comments), reconcile conflict paths.
"""

from __future__ import annotations

import locale as _locale

from skit import i18n, pep723
from skit.langs.python import metawriter, reconcile
from skit.params import ParamDecl

# =====================================================================================
# i18n: 130->134 (system locale lookup returns None) and 215 (ngettext lazy-init)
# =====================================================================================


class TestI18nResidual:
    def test_detect_locale_falls_back_to_empty_when_system_locale_is_none(self, monkeypatch):
        # No env preference anywhere, and locale.getlocale() reports no language code (common in
        # minimal/CI containers running "C"/POSIX with no language set): detect_locale must not
        # raise and must degrade to "" rather than crash on NoneType.
        monkeypatch.delenv("SKIT_LANG", raising=False)
        monkeypatch.delenv("LC_ALL", raising=False)
        monkeypatch.delenv("LC_MESSAGES", raising=False)
        monkeypatch.delenv("LANG", raising=False)
        monkeypatch.setenv("SKIT_CONFIG_DIR", "/nonexistent-skit-config-dir")
        monkeypatch.setattr(_locale, "getlocale", lambda *a, **k: (None, None))
        assert i18n.detect_locale() == ""

    def test_ngettext_lazy_initializes_when_translations_unset(self, monkeypatch):
        # If ngettext is called before gettext/init ever ran (e.g. a plural message is the first
        # i18n call in a process), it must self-initialize rather than raise AttributeError on
        # None.gettext.
        monkeypatch.setattr(i18n, "_translations", None)
        try:
            result = i18n.ngettext("%(n)s file", "%(n)s files", 1)
            assert result.endswith("file")
            assert i18n._translations is not None
        finally:
            i18n.init("en")

    def teardown_method(self):
        i18n.init("en")


# =====================================================================================
# i18n: adversarial locale-negotiation / fallback-chain probing
# =====================================================================================


class TestI18nAdversarial:
    def test_expand_chain_deduplicates_alias_collapsing_to_same_tag(self):
        # zh-Hant-TW -> ["zh-Hant-TW", "zh-Hant", "zh"], and "zh-Hant" already aliases to
        # "zh-TW" which isn't in the chain yet, "zh" has no alias — must not contain dupes.
        chain = i18n._expand_chain("zh-Hant-TW")
        assert len(chain) == len(set(chain))
        assert "zh-Hant-TW" in chain
        assert "zh" in chain

    def test_is_supported_pseudo_locale_case_insensitive(self):
        assert i18n.is_supported("X-Pseudo")
        assert i18n.is_supported("x-pseudo")

    def test_is_supported_unknown_locale_false(self):
        assert not i18n.is_supported("ko-KR")

    def test_is_supported_empty_tag_false(self):
        assert not i18n.is_supported("")

    def test_negotiate_empty_requested_yields_en_only(self):
        primary, chain = i18n.negotiate("")
        assert primary == "en"
        assert chain == ["en"]

    def test_normalize_strips_encoding_and_modifier(self):
        assert i18n._normalize("zh_TW.UTF-8@modifier") == "zh-TW"

    def test_normalize_empty_string(self):
        assert i18n._normalize("") == ""

    def test_config_language_survives_malformed_toml(self, tmp_path, monkeypatch):
        # A corrupted config.toml must not blow up locale detection; it should be treated as "no
        # preference", not raise.
        monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
        (tmp_path / "config.toml").write_text("language = [unterminated\n", encoding="utf-8")
        assert i18n._config_language() == ""

    def test_pseudo_roundtrips_through_init_and_back(self):
        i18n.init("x-pseudo")
        assert i18n.current_locale() == i18n.PSEUDO_LOCALE
        i18n.init("en")
        assert i18n.current_locale() == "en"

    def teardown_method(self):
        i18n.init("en")


# =====================================================================================
# metawriter: 163->165 (empty existing body + new params) and 167->169 (fully emptied block)
# =====================================================================================


class TestMetawriterResidual:
    def test_write_params_into_existing_but_empty_block(self):
        # An existing PEP 723 block whose body is completely empty (e.g. hand-authored with no
        # dependencies comment at all): writing params must not prepend a spurious blank comment
        # separator, since there was nothing to separate from.
        src = "# /// script\n# ///\nprint('hi')\n"
        params = [ParamDecl(name="X", binding="const", type="str", default="v")]
        out = metawriter.write_params(src, params)
        assert [p.name for p in metawriter.read_params(out)] == ["X"]
        # No leading blank "#" separator line before the [tool.skit] header.
        body = out.split("# /// script\n", 1)[1].split("# ///\n", 1)[0]
        assert not body.startswith("#\n")

    def test_write_params_empty_removes_down_to_bare_block(self):
        # Start from a block that contains *only* the [tool.skit] section (no dependencies line),
        # then remove all params: the body must collapse to nothing, leaving a bare block, not a
        # dangling blank comment line.
        src = "# /// script\n# ///\nprint('hi')\n"
        params = [ParamDecl(name="X", binding="const", type="str", default="v")]
        with_params = metawriter.write_params(src, params)
        cleared = metawriter.write_params(with_params, [])
        assert metawriter.read_params(cleared) == []
        assert "# /// script\n# ///\n" in cleared
        assert pep723.has_block(cleared)


# =====================================================================================
# metawriter: adversarial round-trip fidelity (CRLF, unicode, comments, control chars)
# =====================================================================================


class TestMetawriterAdversarial:
    def test_unicode_default_roundtrip(self):
        params = [ParamDecl(name="CITY", binding="const", type="str", default="台北市 🌆")]
        out = metawriter.write_params("x = 1\n", params)
        got = metawriter.read_params(out)
        assert got[0].default == "台北市 🌆"

    def test_crlf_source_preserved_outside_block(self):
        src = "print('a')\r\nprint('b')\r\n"
        params = [ParamDecl(name="X", binding="const", type="str", default="v")]
        out = metawriter.write_params(src, params)
        # user code lines (outside the injected block) must survive untouched, CRLF and all
        assert "print('a')\r\nprint('b')\r\n" in out

    def test_prompt_with_newline_does_not_corrupt_block(self):
        # A prompt containing a literal newline must be escaped so the block still round-trips
        # (an unescaped newline would split the TOML string across two "# " comment lines and
        # corrupt the block).
        params = [ParamDecl(name="X", binding="input", type="str", prompt="Line1\nLine2", order=0)]
        out = metawriter.write_params("x = 1\n", params)
        meta = pep723.parse_block(out)
        assert meta is not None  # still parses as valid TOML-in-comments
        got = metawriter.read_params(out)
        assert got[0].prompt == "Line1\nLine2"

    def test_control_character_in_default_escaped(self):
        params = [ParamDecl(name="X", binding="const", type="str", default="a\x01b")]
        out = metawriter.write_params("x = 1\n", params)
        got = metawriter.read_params(out)
        assert got[0].default == "a\x01b"

    def test_read_params_no_block_returns_empty(self):
        assert metawriter.read_params("print('hi')\n") == []

    def test_read_params_block_without_skit_section_returns_empty(self):
        src = "# /// script\n# dependencies = []\n# ///\n"
        assert metawriter.read_params(src) == []

    def test_multiple_params_blank_line_separated(self):
        params = [
            ParamDecl(name="A", binding="const", type="str", default="1"),
            ParamDecl(name="B", binding="const", type="str", default="2"),
        ]
        out = metawriter.write_params("x = 1\n", params)
        # render_skit_toml puts a blank line before each [[tool.skit.params]] table
        assert out.count("[[tool.skit.params]]") == 2


# =====================================================================================
# pep723: adversarial malformed-block handling
# =====================================================================================


class TestPep723Adversarial:
    def test_parse_block_with_invalid_toml_body_returns_none(self):
        # A block whose stripped body isn't valid TOML (dangling key) must degrade to None, not
        # raise, so callers can treat it as "no metadata" rather than crash.
        src = "# /// script\n# dependencies = \n# ///\n"
        assert pep723.parse_block(src) is None

    def test_has_block_false_for_unterminated_block(self):
        # Missing the closing "# ///" marker: not a valid block.
        src = "# /// script\n# dependencies = []\n"
        assert not pep723.has_block(src)

    def test_inject_block_after_shebang_and_coding_line(self):
        src = "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\nprint('hi')\n"
        out = pep723.inject_block(src, ["requests"])
        lines = out.splitlines()
        assert lines[0] == "#!/usr/bin/env python3"
        assert lines[1] == "# -*- coding: utf-8 -*-"
        assert lines[2] == "# /// script"

    def test_inject_block_noop_when_block_present(self):
        src = "# /// script\n# dependencies = []\n# ///\nprint('hi')\n"
        assert pep723.inject_block(src, ["ignored"]) == src

    def test_suggest_dependencies_excludes_relative_imports(self):
        # `from . import foo` / `from .. import bar` are relative (level != 0) and must never be
        # suggested as third-party dependencies to add to the PEP 723 block.
        src = "from . import sibling\nfrom .. import cousin\nimport requests\n"
        assert pep723.suggest_dependencies(src) == ["requests"]

    def test_suggest_dependencies_excludes_stdlib_and_underscored(self):
        src = "import os\nimport json\nimport __future__\nimport httpx\n"
        assert pep723.suggest_dependencies(src) == ["httpx"]

    def test_set_dependencies_empty_clears_to_bracketless_form(self):
        src = '# /// script\n# dependencies = [\n#     "requests",\n# ]\n# ///\nimport requests\n'
        out = pep723.set_dependencies(src, [])
        meta = pep723.parse_block(out)
        assert meta is not None
        assert meta["dependencies"] == []
        assert "# dependencies = []" in out

    def test_build_block_no_deps_no_requires_python(self):
        block = pep723.build_block([])
        assert block == "# /// script\n# dependencies = []\n# ///\n"


# =====================================================================================
# reconcile: line 159 (remove: name not managed) + adversarial edit_specs interactions
# =====================================================================================


class TestReconcileResidual:
    def test_edit_specs_remove_unmanaged_name_warns(self):
        text = 'CITY = "Taipei"\n'
        specs = [ParamDecl(name="CITY", binding="const", type="str")]
        result = reconcile.edit_specs(text, specs, remove=["GONE"])
        assert result.warnings == ["not-managed:GONE"]
        # the managed one is untouched
        assert [s.name for s in result.specs] == ["CITY"]


class TestReconcileAdversarial:
    def test_edit_specs_remove_then_add_same_name_readmits_from_source(self):
        # Remove CITY, then re-add it in the same call: it should come back freshly derived from
        # the current script (a legitimate "reset a definition" workflow), not error out because
        # it was "already managed" (it no longer is, by the time add runs).
        text = 'CITY = "Osaka"\n'
        specs = [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")]
        result = reconcile.edit_specs(text, specs, remove=["CITY"], add=["CITY"])
        assert result.warnings == []
        assert [s.name for s in result.specs] == ["CITY"]
        assert result.specs[0].default == "Osaka"

    def test_edit_specs_add_already_managed_warns_and_keeps_original(self):
        text = 'CITY = "Osaka"\n'
        specs = [ParamDecl(name="CITY", binding="const", type="str", default="Taipei")]
        result = reconcile.edit_specs(text, specs, add=["CITY"])
        assert result.warnings == ["already-managed:CITY"]
        # original definition (default="Taipei") preserved, not clobbered by the source's current
        # value
        assert result.specs[0].default == "Taipei"

    def test_edit_specs_add_not_a_candidate_warns(self):
        text = 'CITY = "Osaka"\n'
        result = reconcile.edit_specs(text, [], add=["NOPE"])
        assert result.warnings == ["not-a-candidate:NOPE"]
        assert result.specs == []

    def test_edit_specs_resync_prunes_and_retypes_together(self):
        text = 'RETRIES = "3"\n'  # type changed from int to str; CITY variable gone entirely
        specs = [
            ParamDecl(name="CITY", binding="const", type="str", default="Taipei"),
            ParamDecl(name="RETRIES", binding="const", type="int", default=3),
        ]
        result = reconcile.edit_specs(text, specs, resync=True)
        assert result.warnings == ["resync-dropped:CITY"]
        assert [s.name for s in result.specs] == ["RETRIES"]
        assert result.specs[0].type == "str"

    def test_edit_specs_apply_order_resync_then_remove_then_add_then_tweak(self):
        # Documented fixed apply order: resync -> remove -> add -> secret/no_secret/prompt tweaks.
        # Exercise all four categories in one call and check the final state reflects that order.
        text = 'CITY = "Taipei"\nTOKEN = "abc"\n'
        specs = [
            ParamDecl(name="GONE", binding="const", type="str", default="x"),  # dropped by resync
            ParamDecl(name="CITY", binding="const", type="str", default="Taipei"),
        ]
        result = reconcile.edit_specs(
            text,
            specs,
            resync=True,
            remove=["CITY"],
            add=["TOKEN"],
            secret=["TOKEN"],
            prompts={"TOKEN": "Token: "},
        )
        assert [s.name for s in result.specs] == ["TOKEN"]
        assert result.specs[0].secret is True
        assert result.specs[0].prompt == "Token: "
        assert "resync-dropped:GONE" in result.warnings

    def test_render_warning_all_known_codes(self):
        for code, name in [
            ("not-managed", "X"),
            ("resync-dropped", "Y"),
            ("already-managed", "Z"),
            ("not-a-candidate", "W"),
        ]:
            text = reconcile.render_warning(f"{code}:{name}")
            assert name in text

    def test_reconcile_const_and_input_conflict_together(self):
        # Both an input and const drift simultaneously in one reconcile call.
        script = 'name = input("Name: ")\nprint(name)\n'  # RETRIES const is gone; input kept
        specs = [
            ParamDecl(name="RETRIES", binding="const", type="int", default=3),
            ParamDecl(name="input-1", binding="input", type="str", order=0),
        ]
        report = reconcile.reconcile(script, specs)
        assert [s.name for s in report.missing] == ["RETRIES"]
        assert [s.name for s in report.ok] == ["input-1"]
        assert report.has_drift
