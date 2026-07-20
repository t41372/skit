"""Mutation-kill tests for src/skit/i18n.py — the script-aware "zh" fallback (`_zh_family`,
`_expand_chain`) and the exact corrupt-config warning text emitted by `set_language`.

These exercise the real translation layer: locale negotiation through the public `negotiate` /
`is_supported` surface, and the English-source warning strings (English is the identity catalog,
so `gettext(msgid)` returns the msgid — the exact text asserted here).
"""

from __future__ import annotations

from skit import atomic, i18n


def teardown_function() -> None:
    # Every test may re-init the global translations; restore the English default so this file
    # never leaks a locale into whatever runs next.
    i18n.init("en")


# ---- script-aware macrolanguage fallback (_expand_chain / _zh_family) ----------


def test_bare_zh_step_resolves_to_script_family_not_none() -> None:
    """When a qualified Traditional tag is truncated down to the bare "zh" macrolanguage subtag,
    that step must alias to the tag's script family (zh-TW), not be dropped. `zh-Latn-TW` reaches
    zh-TW ONLY via this family step (none of its truncations alias to zh-TW otherwise), so it is a
    clean probe: with `alias = family` the negotiated locale is zh-TW; with the mutant `alias =
    None` the family is lost and negotiation falls all the way back to English."""
    assert i18n.negotiate("zh-Latn-TW") == ("zh-TW", ["zh-TW", "en"])
    assert i18n.is_supported("zh-Latn-TW") is True


def test_zh_family_none_for_non_zh_tag_carrying_a_script_hint() -> None:
    """`_zh_family` is only meaningful for zh-rooted tags: a non-"zh" tag returns None even when it
    carries a subtag that WOULD be a Hant/Hans hint on a Chinese tag (e.g. `en-Hant`, `de-HK`). The
    guard is `parts[0] != "zh" or len(parts) == 1`; the mutant `... and ...` lets a two-part
    non-zh tag fall through to the hint checks and mis-classify it as Chinese."""
    assert i18n._zh_family("en-Hant") is None
    assert i18n._zh_family("de-HK") is None
    # sanity: the hints themselves do classify a genuine zh tag (proves the checks are reachable)
    assert i18n._zh_family("zh-Hant") == "zh-TW"


# ---- set_language: exact corrupt-config warning text ----------


def test_corrupt_config_backed_up_warning_exact_text(tmp_path, monkeypatch, capsys) -> None:
    """When a corrupt config.toml is backed up before being overwritten, the user is warned on
    stderr with the FULL English message — naming the file, the backup, and the recovery hint.
    Asserting the exact rendered text kills the string mutants on this message: XX-wrapping either
    literal, lower-casing "It", and substituting str(None) for the real path."""
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
    monkeypatch.setenv("SKIT_LANG", "en")
    monkeypatch.delenv("LC_ALL", raising=False)
    monkeypatch.delenv("LC_MESSAGES", raising=False)
    i18n.init("en")

    path = tmp_path / "config.toml"
    path.write_text("this is = = not valid toml", encoding="utf-8")

    effective = i18n.set_language("en")
    assert effective == "en"

    backup = tmp_path / "config.toml.bak"
    assert backup.is_file()

    err = capsys.readouterr().err
    expected = (
        f"{path} is corrupt and could not be parsed. It has been backed up to "
        f"{backup} before this change; recover any lost settings from that file."
    )
    assert expected in err


def test_corrupt_config_unbackable_warning_exact_text(tmp_path, monkeypatch, capsys) -> None:
    """When the corrupt config can't even be backed up (the copy fails), the OTHER message is
    emitted — data-loss is imminent. Asserting the exact rendered text kills the string mutants on
    this else-branch message: XX-wrapping either literal, and upper-casing the second sentence."""
    monkeypatch.setenv("SKIT_CONFIG_DIR", str(tmp_path))
    monkeypatch.setenv("SKIT_LANG", "en")
    monkeypatch.delenv("LC_ALL", raising=False)
    monkeypatch.delenv("LC_MESSAGES", raising=False)
    i18n.init("en")

    path = tmp_path / "config.toml"
    path.write_text("this is = = not valid toml", encoding="utf-8")

    def _boom(*_a, **_k):
        raise OSError("disk full")

    monkeypatch.setattr(atomic.shutil, "copy2", _boom)

    effective = i18n.set_language("en")
    assert effective == "en"
    assert not (tmp_path / "config.toml.bak").exists()

    err = capsys.readouterr().err
    expected = (
        f"{path} is corrupt and could not be parsed, and it could not be backed up "
        "either; the settings it contained will be lost when this change is saved."
    )
    assert expected in err
