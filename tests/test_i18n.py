"""i18n tests: catalog parity, locale negotiation, fallback, plurals, pseudo-locale."""

from __future__ import annotations

import tomllib
from pathlib import Path

import pytest
from babel.messages.catalog import Catalog
from babel.messages.mofile import write_mo
from babel.messages.pofile import read_po

from skit import atomic, i18n

LOCALES_DIR = Path(i18n.__file__).parent / "locales"
POT = LOCALES_DIR / "skit.pot"
TRANSLATED = [p.name for p in LOCALES_DIR.iterdir() if (p / "LC_MESSAGES" / "skit.po").is_file()]


def _msgids(po_path: Path) -> set[str]:
    """The set of source msgids in a .po/.pot (plural entries keyed by their singular)."""
    with po_path.open("rb") as f:
        catalog = read_po(f)
    # a plural message's id is a (singular, plural) sequence — key it by the singular
    return {(m.id if isinstance(m.id, str) else m.id[0]) for m in catalog if m.id}


class TestCatalogParity:
    """The .pot (English source) is the source of truth; every shipped .po must cover the same
    msgids — no missing translations, no orphans."""

    def test_locales_shipped(self):
        assert "en" in i18n.available_locales()
        assert len(i18n.available_locales()) >= 3

    @pytest.mark.parametrize("locale", TRANSLATED)
    def test_parity_with_pot(self, locale: str):
        pot_ids = _msgids(POT)
        po_ids = _msgids(LOCALES_DIR / locale / "LC_MESSAGES" / "skit.po")
        missing = pot_ids - po_ids
        extra = po_ids - pot_ids
        assert not missing, f"{locale} is missing translations: {sorted(missing)}"
        assert not extra, f"{locale} has msgids not in the .pot (orphans): {sorted(extra)}"

    def test_pot_covers_all_source_gettext_msgids(self):
        # Guard the blind spot test_parity_with_pot can't see: a NEW gettext() string in the source
        # that was never extracted into the .pot would silently ship untranslated (English fallback
        # for every locale). Extract straight from src/skit the same way scripts/i18n.py does
        # (babel.cfg == all **.py) and assert every source msgid is present in the committed .pot.
        from babel.messages.extract import extract_from_dir

        # Anchor on the real repo tree, not the imported package and not this file's own
        # location: under mutmut the whole project (src AND tests) is copied into the
        # generated mutants/ tree, whose mutant function bodies contain a variant of
        # every string literal — extracting from there floods this parity check with
        # mutant msgids that were never (and should never be) in the .pot. The check's
        # subject is the committed source, which is exactly what scripts/i18n.py extract
        # reads — so when this test itself runs from inside mutants/, hop back out.
        root = Path(__file__).resolve().parent.parent
        if "mutants" in root.parts:
            root = Path(*root.parts[: root.parts.index("mutants")])
        src_dir = root / "src" / "skit"
        source_ids: set[str] = set()
        for _fn, _lineno, message, _comments, _ctx in extract_from_dir(
            src_dir, method_map=[("**.py", "python")]
        ):
            # ngettext yields a (singular, plural) tuple — key by the singular, as _msgids does.
            if isinstance(message, tuple):
                if message[0]:
                    source_ids.add(message[0])
            elif message:
                source_ids.add(message)
        missing = source_ids - _msgids(POT)
        assert not missing, (
            "source gettext msgids missing from skit.pot "
            f"(run scripts/i18n.py extract): {sorted(missing)}"
        )


class TestNegotiation:
    def test_exact_match(self):
        assert i18n.negotiate("zh-TW")[0] == "zh-TW"

    def test_alias_hant(self):
        # zh-HK / zh-Hant both map to Traditional Chinese
        assert i18n.negotiate("zh-HK")[0] == "zh-TW"
        assert i18n.negotiate("zh-Hant")[0] == "zh-TW"

    def test_alias_hans_and_bare_zh(self):
        assert i18n.negotiate("zh")[0] == "zh-CN"
        assert i18n.negotiate("zh-SG")[0] == "zh-CN"

    def test_posix_style_tag(self):
        assert i18n.negotiate(i18n._normalize("zh_TW.UTF-8"))[0] == "zh-TW"

    def test_unknown_falls_back_to_en(self):
        primary, chain = i18n.negotiate("ko-KR")
        assert primary == "en"
        assert chain[-1] == "en"

    def test_chain_always_ends_with_en(self):
        for tag in ("zh-TW", "zh", "ja", ""):
            assert i18n.negotiate(tag)[1][-1] == "en"

    def test_traditional_chain_excludes_simplified(self):
        # Regression: the bare "zh" macrolanguage fallback used to always alias to zh-CN, so every
        # Traditional tag's negotiated chain smuggled Simplified in ahead of English. A Traditional
        # tag's chain must go straight to en if zh-TW itself doesn't cover a msgid.
        for tag in ("zh-TW", "zh-HK", "zh-MO", "zh-Hant"):
            primary, chain = i18n.negotiate(tag)
            assert primary == "zh-TW"
            assert "zh-CN" not in chain
            assert chain == ["zh-TW", "en"]

    def test_simplified_chain_still_resolves_to_zh_cn(self):
        # Preserve existing correct behavior: Simplified-family tags (and the bare "zh" request,
        # which conventionally means Simplified) are unaffected by the script-aware fallback fix.
        for tag in ("zh-CN", "zh-Hans", "zh-SG", "zh-MY", "zh"):
            primary, chain = i18n.negotiate(tag)
            assert primary == "zh-CN"
            assert chain == ["zh-CN", "en"]

    def test_zh_region_with_no_script_hint_defaults_to_simplified(self):
        # A "zh-*" tag whose region/script subtag isn't one of the known Hant/Hans hints (e.g. a
        # region skit doesn't special-case) has no inferable family: _zh_family returns None, so
        # the bare "zh" fallback step keeps its pre-fix, unconditional default of zh-CN.
        assert i18n._zh_family("zh-XX") is None
        primary, chain = i18n.negotiate("zh-XX")
        assert primary == "zh-CN"
        assert chain == ["zh-CN", "en"]

    def test_conflicting_script_and_region_lets_script_win(self):
        # Regression (asymmetric mirror of the bug just fixed): a tag can carry an explicit script
        # subtag alongside a region subtag associated with the *other* script family (both are
        # valid CLDR locale identifiers). BCP-47 treats script as more specific than region, so
        # script must decide the family, not region — otherwise the cross-script leak the
        # script-aware fallback fix eliminated reappears in the mirror direction.
        assert i18n._zh_family("zh-Hant-CN") == "zh-TW"
        assert i18n._zh_family("zh-Hans-TW") == "zh-CN"

        # zh-Hant-CN: explicit Traditional script wins over the Simplified-associated "CN" region —
        # bins Traditional, chain excludes zh-CN and falls straight to en.
        primary, chain = i18n.negotiate("zh-Hant-CN")
        assert primary == "zh-TW"
        assert chain == ["zh-TW", "en"]
        assert "zh-CN" not in chain

        # zh-Hans-TW: explicit Simplified script wins over the Traditional-associated "TW" region —
        # bins Simplified, resolves to zh-CN.
        primary, chain = i18n.negotiate("zh-Hans-TW")
        assert primary == "zh-CN"
        assert chain == ["zh-CN", "en"]
        assert "zh-TW" not in chain

    def test_conflicting_script_and_region_hk_mo_variants(self):
        # Same precedence rule, exercised over the other Traditional-region hints (hk, mo) paired
        # with an explicit Hans script subtag — all are valid, reachable CLDR locale identifiers.
        for tag in ("zh-Hans-HK", "zh-Hans-MO"):
            assert i18n._zh_family(tag) == "zh-CN"
            primary, chain = i18n.negotiate(tag)
            assert primary == "zh-CN"
            assert chain == ["zh-CN", "en"]

    def test_plain_tags_unaffected_by_script_over_region_precedence(self):
        # Regression: the already-correct non-conflicting cases (script and region agree, or only
        # one of them is present) must be completely unchanged by the script-over-region fix.
        assert i18n._zh_family("zh-TW") == "zh-TW"
        assert i18n._zh_family("zh-HK") == "zh-TW"
        assert i18n._zh_family("zh-Hant") == "zh-TW"
        assert i18n._zh_family("zh-CN") == "zh-CN"
        assert i18n._zh_family("zh-Hans") == "zh-CN"
        assert i18n._zh_family("zh") is None

        for tag in ("zh-TW", "zh-HK", "zh-Hant"):
            primary, chain = i18n.negotiate(tag)
            assert primary == "zh-TW"
            assert chain == ["zh-TW", "en"]

        for tag in ("zh-CN", "zh-Hans", "zh"):
            primary, chain = i18n.negotiate(tag)
            assert primary == "zh-CN"
            assert chain == ["zh-CN", "en"]


class TestFormatting:
    def test_zh_tw_message(self):
        i18n.init("zh-TW")
        assert i18n.gettext("Name") == "名稱"

    def test_zh_cn_message(self):
        i18n.init("zh-CN")
        assert i18n.gettext("Name") == "名称"

    def test_en_plural(self):
        i18n.init("en")
        sing, plur = "%(shown)s/%(total)s script", "%(shown)s/%(total)s scripts"
        one = i18n.ngettext(sing, plur, 1)
        many = i18n.ngettext(sing, plur, 5)
        assert one.endswith("script")
        assert not one.endswith("scripts")
        assert many.endswith("scripts")

    def test_zh_plural_single_form(self):
        # Chinese has one plural form (nplurals=1): singular and plural render identically.
        i18n.init("zh-CN")
        sing, plur = "%(shown)s/%(total)s script", "%(shown)s/%(total)s scripts"
        assert i18n.ngettext(sing, plur, 1) == i18n.ngettext(sing, plur, 9)

    def test_variable_substitution(self):
        i18n.init("en")
        msg = i18n.gettext("%(file)s isn't a .py file — pass --exe if it's an executable")
        assert "photo.py" in msg % {"file": "photo.py"}

    def test_missing_id_returns_source(self):
        i18n.init("en")
        assert i18n.gettext("no-such-message") == "no-such-message"

    def test_pseudo_locale(self):
        i18n.init("x-pseudo")
        text = i18n.gettext("Name")
        assert text.startswith("⟦")
        assert text.endswith("⟧")
        assert "à" in text or "é" in text.lower() or "Nàmé" in text

    def test_pseudo_preserves_placeholder(self):
        # %-placeholders must survive the pseudo transform so call-site substitution still works.
        i18n.init("x-pseudo")
        text = i18n.gettext("%(file)s isn't a .py file — pass --exe if it's an executable")
        assert "%(file)s" in text
        assert "photo.py" in text % {"file": "photo.py"}

    def teardown_method(self):
        i18n.init("en")


def _write_mo(path: Path, locale: str, messages: dict[str, str]) -> None:
    """Compile a minimal synthetic .mo catalog for use as an isolated fake locales dir."""
    path.parent.mkdir(parents=True, exist_ok=True)
    catalog = Catalog(locale=locale)
    for msgid, msgstr in messages.items():
        catalog.add(msgid, msgstr)
    with path.open("wb") as f:
        write_mo(f, catalog)


class TestScriptAwareFallback:
    """Regression for the bug where every Traditional tag's negotiated chain smuggled in zh-CN
    (Simplified) as a fallback ahead of English, because the bare "zh" macrolanguage step always
    aliased to zh-CN regardless of the requested tag's script. Uses synthetic catalogs (rather than
    the real, currently-100%-complete zh_TW/zh_CN catalogs) so a genuinely missing translation can
    be exercised deterministically."""

    def _fake_locales(self, tmp_path: Path) -> Path:
        locales = tmp_path / "locales"
        # zh_TW: translates "Name" but not "New String" (simulates a staggered translation update).
        _write_mo(locales / "zh_TW" / "LC_MESSAGES" / "skit.mo", "zh_TW", {"Name": "名稱"})
        # zh_CN: has both, including a Simplified translation of "New String" that must never leak
        # into a zh-TW user's output.
        _write_mo(
            locales / "zh_CN" / "LC_MESSAGES" / "skit.mo",
            "zh_CN",
            {"Name": "名称", "New String": "新字符串"},
        )
        return locales

    def test_missing_zh_tw_msgid_falls_back_to_english_not_simplified(self, tmp_path, monkeypatch):
        monkeypatch.setattr(i18n, "_LOCALES_DIR", self._fake_locales(tmp_path))
        i18n.init("zh-TW")
        assert i18n.gettext("Name") == "名稱"  # translated in zh_TW: used as-is
        assert i18n.gettext("New String") == "New String"  # missing in zh_TW: English source,
        # NOT the zh_CN Simplified translation "新字符串"

    def test_missing_zh_hk_msgid_falls_back_to_english_not_simplified(self, tmp_path, monkeypatch):
        # zh-HK aliases to the zh-TW catalog; same script-family guarantee applies.
        monkeypatch.setattr(i18n, "_LOCALES_DIR", self._fake_locales(tmp_path))
        i18n.init("zh-HK")
        assert i18n.gettext("New String") == "New String"

    def test_zh_cn_still_gets_its_own_translation(self, tmp_path, monkeypatch):
        # Unaffected: Simplified resolution to zh-CN must still work exactly as before.
        monkeypatch.setattr(i18n, "_LOCALES_DIR", self._fake_locales(tmp_path))
        i18n.init("zh-CN")
        assert i18n.gettext("New String") == "新字符串"

    def test_conflicting_script_and_region_tag_end_to_end(self, tmp_path, monkeypatch):
        # Regression, mirror direction: zh-Hans-HK carries an explicit Simplified script subtag but
        # a Traditional-associated "HK" region. Script must win, so this negotiates to zh-CN and a
        # msgid missing from zh_CN.mo must fall through to the English source — NOT leak the
        # Traditional translation from zh_TW.mo (which is what the pre-fix region-first check did).
        monkeypatch.setattr(i18n, "_LOCALES_DIR", self._fake_locales(tmp_path))
        i18n.init("zh-Hans-HK")
        assert i18n.current_locale() == "zh-CN"
        assert i18n.gettext("New String") == "新字符串"  # zh-CN has its own translation
        assert i18n.gettext("Name") == "名称"  # zh-CN's Simplified glyph, not zh-TW's "名稱"

    def teardown_method(self):
        i18n.init("en")


class TestDetection:
    def test_env_override_wins(self, monkeypatch):
        monkeypatch.setenv("SKIT_LANG", "zh-TW")
        monkeypatch.setenv("LANG", "en_US.UTF-8")
        assert i18n.detect_locale() == "zh-TW"

    def test_lang_env(self, monkeypatch):
        monkeypatch.delenv("SKIT_LANG", raising=False)
        monkeypatch.delenv("LC_ALL", raising=False)
        monkeypatch.delenv("LC_MESSAGES", raising=False)
        monkeypatch.setenv("LANG", "zh_CN.UTF-8")
        assert i18n.detect_locale() == "zh-CN"

    def test_c_locale_ignored(self, monkeypatch):
        monkeypatch.delenv("SKIT_LANG", raising=False)
        monkeypatch.setenv("LC_ALL", "C")
        monkeypatch.delenv("LC_MESSAGES", raising=False)
        monkeypatch.setenv("LANG", "C")
        # The C locale is not a language preference; result depends on the system locale —
        # just ensure it doesn't raise.
        i18n.detect_locale()

    def test_set_language_persists(self, tmp_path, monkeypatch):
        monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
        monkeypatch.delenv("SKIT_LANG", raising=False)
        effective = i18n.set_language("zh-TW")
        assert effective == "zh-TW"
        assert (tmp_path / "config.toml").is_file()
        assert i18n._config_language() == "zh-TW"
        i18n.set_language("")
        assert i18n._config_language() == ""

    def teardown_method(self):
        i18n.init("en")


# --- corrupt config.toml must never be silently wiped by set_language (matches config.py's
# save_editor/save_mirror recovery, via the shared atomic.load_toml_recoverable helper) ---


class TestSetLanguageCorruptConfig:
    def test_backs_up_corrupt_config_instead_of_wiping_it(self, tmp_path, monkeypatch, capsys):
        monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
        monkeypatch.delenv("SKIT_LANG", raising=False)
        corrupt = (
            'language = "zh-CN"\n[mirror]\nenabled = true\npypi = "https://tsinghua"\n'
            "this is = = not valid toml"
        )
        (tmp_path / "config.toml").write_text(corrupt, encoding="utf-8")

        effective = i18n.set_language("en")

        # the just-requested change still takes effect...
        assert effective == "en"
        assert i18n._config_language() == "en"
        # ...but the corrupt original is preserved verbatim in a backup rather than vanishing.
        backup = tmp_path / "config.toml.bak"
        assert backup.is_file()
        assert backup.read_text(encoding="utf-8") == corrupt
        # ...and the user is told on stderr, so the data loss (of the [mirror] section, which
        # cannot be recovered from the corrupt file itself) isn't silent.
        err = capsys.readouterr().err
        assert "config.toml" in err
        assert "config.toml.bak" in err

    def test_warns_when_corrupt_config_cannot_even_be_backed_up(
        self, tmp_path, monkeypatch, capsys
    ):
        monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
        monkeypatch.delenv("SKIT_LANG", raising=False)
        (tmp_path / "config.toml").write_text("this is = = not valid toml", encoding="utf-8")

        def boom(*_a, **_k):
            raise OSError("disk full")

        monkeypatch.setattr(atomic.shutil, "copy2", boom)
        effective = i18n.set_language("en")

        assert effective == "en"
        assert i18n._config_language() == "en"
        assert not (tmp_path / "config.toml.bak").exists()
        err = capsys.readouterr().err
        assert "config.toml" in err

    def test_valid_config_is_unaffected(self, tmp_path, monkeypatch):
        # Regression: the recovery path must not kick in (and must not create a .bak) for an
        # ordinary, parseable config — the [mirror] section must survive the language change.
        monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
        monkeypatch.delenv("SKIT_LANG", raising=False)
        (tmp_path / "config.toml").write_text(
            'language = "zh-CN"\n[mirror]\nenabled = true\npypi = "https://tsinghua"\n',
            encoding="utf-8",
        )

        i18n.set_language("en")

        assert i18n._config_language() == "en"
        with open(tmp_path / "config.toml", "rb") as f:
            doc = tomllib.load(f)
        assert doc["mirror"] == {"enabled": True, "pypi": "https://tsinghua"}
        assert not (tmp_path / "config.toml.bak").exists()

    def teardown_method(self):
        i18n.init("en")
