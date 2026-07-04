"""i18n tests: catalog parity, locale negotiation, fallback, plurals, pseudo-locale."""

from __future__ import annotations

from pathlib import Path

import pytest
from babel.messages.pofile import read_po

from skit import i18n

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
