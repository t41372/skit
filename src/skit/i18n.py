"""i18n core (GNU gettext).

Design notes:
- Catalogs are standard gettext with **source-string msgids**: the English text is the msgid, so an
  untranslated entry falls back to the English source automatically. English therefore ships no .mo
  (it is the identity). Translations live at locales/<gettext_locale>/LC_MESSAGES/skit.mo — POSIX
  underscore dirs (zh_CN), while the public tags stay hyphenated (zh-CN).
- Edit the .po sources, then recompile with `python scripts/i18n.py compile` (Babel). Runtime uses the
  stdlib `gettext` module only — zero third-party i18n dependencies.
- Locale negotiation chain: SKIT_LANG > config.toml [language] > LC_ALL > LC_MESSAGES > LANG > system
  locale > en. Each candidate expands into a fallback chain (zh-TW -> zh-Hant alias -> zh -> en),
  wired into gettext via its language list so a missing translation falls back entry by entry and
  always ends at the English source.
- Pseudo-locale (x-pseudo): an ⟦bracket + stretch⟧ transform over English, used to visually catch
  hard-coded / untranslated strings and truncation. Placeholders (%(name)s) are preserved so
  %-substitution at the call site still works.
- Headless: no CLI/TUI dependency; store/launcher can import it safely.
"""

from __future__ import annotations

import gettext as _gt
import os
import re
import tomllib
from pathlib import Path
from typing import Any

from .atomic import atomic_write_toml
from .paths import config_dir

_LOCALES_DIR = Path(__file__).parent / "locales"
_DOMAIN = "skit"
DEFAULT_LOCALE = "en"
PSEUDO_LOCALE = "x-pseudo"

# Alias -> the locale we actually ship.
_ALIASES = {
    "zh": "zh-CN",
    "zh-hans": "zh-CN",
    "zh-sg": "zh-CN",
    "zh-my": "zh-CN",
    "zh-hant": "zh-TW",
    "zh-hk": "zh-TW",
    "zh-mo": "zh-TW",
}

_translations: _gt.NullTranslations | None = None
_active: str = DEFAULT_LOCALE
_pseudo: bool = False


def _gt_dir(tag: str) -> str:
    """Public hyphen tag -> gettext locale dir (POSIX underscore): 'zh-CN' -> 'zh_CN'."""
    return tag.replace("-", "_")


def available_locales() -> list[str]:
    """Locales we ship: English (identity) plus every dir with a compiled catalog."""
    found = {DEFAULT_LOCALE}
    if _LOCALES_DIR.is_dir():
        for p in _LOCALES_DIR.iterdir():
            if (p / "LC_MESSAGES" / f"{_DOMAIN}.mo").is_file():
                found.add(p.name.replace("_", "-"))
    return sorted(found)


def _normalize(tag: str) -> str:
    """'zh_TW.UTF-8' -> 'zh-TW'; normalizes casing."""
    tag = tag.split(".", maxsplit=1)[0].split("@", maxsplit=1)[0].replace("_", "-").strip()
    if not tag:
        return ""
    parts = tag.split("-")
    out = [parts[0].lower()]
    for p in parts[1:]:
        if len(p) == 2:
            out.append(p.upper())
        elif len(p) == 4:
            out.append(p.capitalize())
        else:
            out.append(p.lower())
    return "-".join(out)


def _expand_chain(tag: str) -> list[str]:
    """Expand a single tag into a fallback chain: zh-Hant-TW -> [zh-Hant-TW, zh-Hant, zh] + alias
    resolution."""
    chain: list[str] = []
    parts = tag.split("-")
    while parts:
        cand = "-".join(parts)
        alias = _ALIASES.get(cand.lower(), cand)
        for c in (cand, alias):
            if c not in chain:
                chain.append(c)
        parts.pop()
    return chain


def _config_language() -> str:
    path = config_dir() / "config.toml"
    if not path.is_file():
        return ""
    try:
        with open(path, "rb") as f:
            return str(tomllib.load(f).get("language", ""))
    except (OSError, tomllib.TOMLDecodeError):
        return ""


def detect_locale() -> str:
    """Return the raw tag the user requested (un-negotiated).

    An empty string means no preference was found.
    """
    for source in (
        os.environ.get("SKIT_LANG", ""),
        _config_language(),
        os.environ.get("LC_ALL", ""),
        os.environ.get("LC_MESSAGES", ""),
        os.environ.get("LANG", ""),
    ):
        tag = _normalize(source) if source and source.lower() != "c" else ""
        if tag:
            return tag
    try:
        import locale as _locale

        loc = _locale.getlocale()[0]
        if loc:
            return _normalize(loc)
    except (ValueError, TypeError):
        pass
    return ""


def is_supported(tag: str) -> bool:
    """Whether the tag (or any candidate on its fallback chain) maps to a shipped locale.
    The pseudo-locale is always considered supported. Used to validate entry points like the CLI
    `lang` command."""
    normalized = _normalize(tag)
    if not normalized:
        return False
    if normalized.lower() == PSEUDO_LOCALE:
        return True
    shipped = set(available_locales())
    return any(c in shipped for c in _expand_chain(normalized))


def negotiate(requested: str) -> tuple[str, list[str]]:
    """Negotiate (primary locale, full fallback chain ending in en)."""
    shipped = set(available_locales())
    chain: list[str] = []
    if requested:
        for cand in _expand_chain(requested):
            if cand in shipped and cand not in chain:
                chain.append(cand)
    if DEFAULT_LOCALE not in chain:
        chain.append(DEFAULT_LOCALE)
    return chain[0], chain


def init(lang: str | None = None) -> str:
    """Initialize (or reinitialize) the message system; return the effective primary locale."""
    global _translations, _active, _pseudo
    requested = _normalize(lang) if lang else detect_locale()
    _pseudo = requested.lower() == PSEUDO_LOCALE
    if _pseudo:
        primary, chain = DEFAULT_LOCALE, [DEFAULT_LOCALE]
        _active = PSEUDO_LOCALE
    else:
        primary, chain = negotiate(requested)
        _active = primary
    # English is the msgid itself (identity), so only non-en locales need a catalog; gettext chains
    # them as fallbacks in order and returns the English source for anything still untranslated.
    languages = [_gt_dir(c) for c in chain if c != DEFAULT_LOCALE]
    _translations = _gt.translation(_DOMAIN, _LOCALES_DIR, languages=languages, fallback=True)
    return _active


def current_locale() -> str:
    return _active


_PSEUDO_TABLE = str.maketrans("aeiouAEIOU", "àéîöûÀÉÎÖÛ")
_PLACEHOLDER = re.compile(r"%\([^)]*\)[sdr]|%[sdrifgeExXoc%]")


def _pseudoize(text: str) -> str:
    """en -> pseudo: bracket markers + vowel transforms (~30% inflation), leaving %(...)s
    placeholders untouched so call-site %-substitution still resolves."""
    parts: list[str] = []
    last = 0
    for m in _PLACEHOLDER.finditer(text):
        parts.append(text[last : m.start()].translate(_PSEUDO_TABLE))
        parts.append(m.group())
        last = m.end()
    parts.append(text[last:].translate(_PSEUDO_TABLE))
    return f"⟦{''.join(parts)}~~⟧"


def gettext(message: str) -> str:
    """Translate a source-string message (falls back to the English source; never raises)."""
    if _translations is None:
        init()
    if _translations is None:  # pragma: no cover — init() guarantees it is set
        raise RuntimeError("i18n init failed")
    text = _translations.gettext(message)
    return _pseudoize(text) if _pseudo else text


def ngettext(singular: str, plural: str, n: int) -> str:
    """Translate a source-string message with plural selection (CLDR rules per the target locale)."""
    if _translations is None:
        init()
    if _translations is None:  # pragma: no cover — init() guarantees it is set
        raise RuntimeError("i18n init failed")
    text = _translations.ngettext(singular, plural, n)
    return _pseudoize(text) if _pseudo else text


def set_language(tag: str) -> str:
    """Write config.toml and take effect immediately. Returns the effective locale. An empty tag
    clears the setting (back to auto-detection)."""
    path = config_dir() / "config.toml"
    doc: dict[str, Any] = {}
    if path.is_file():
        try:
            with open(path, "rb") as f:
                doc = tomllib.load(f)
        except (OSError, tomllib.TOMLDecodeError):
            doc = {}
    if tag:
        doc["language"] = _normalize(tag)
    else:
        doc.pop("language", None)
    atomic_write_toml(path, doc)
    return init(tag or None)
