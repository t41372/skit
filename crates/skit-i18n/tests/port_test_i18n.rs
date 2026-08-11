//! Mechanical port of the Python oracle module `tests/test_i18n.py`
//! (`origin/main@206f9ef`): "i18n tests: catalog parity, locale negotiation, fallback,
//! plurals, pseudo-locale." Each `#[test]` keeps its Python `def test_*` name so it traces
//! back to its origin, and each Python "WHY"/rationale comment is preserved above it.
//!
//! ARCHITECTURE NOTE — the two i18n layers are built differently, and that difference decides
//! most of the buckets below:
//! - Python `skit.i18n` is a runtime GNU-gettext engine: `.po`/`.mo` catalogs loaded from a
//!   `locales/` dir, a mutable global (`init`/`current_locale`), a negotiation CHAIN (a list of
//!   candidate locales ending in `en`), `ngettext` plural selection, env-var precedence
//!   (SKIT_LANG > config.toml > LC_ALL > LC_MESSAGES > LANG > system), and `set_language`
//!   persistence to `config.toml`.
//! - Rust `skit-i18n` is a compiled STATIC catalog: one `Translation` row carries all three
//!   strings (`english`/`zh_cn`/`zh_tw`), the locale is a per-call `Locale` parameter (no
//!   global, no `init`, so Python's `init()`/`teardown_method` have no analog), a miss falls
//!   back to the English source inside `text`/`render`, and there is no `negotiate` chain, no
//!   `ngettext`, no `.po`/`.mo`, and no config persistence in this crate.
//!
//! Concept mapping used throughout:
//! - Python `i18n.negotiate(tag)[0]` (primary locale) -> `detect_locale(Some(tag))`
//!   (the single resolved `Locale`; `.tag()` gives the public string).
//! - Python `i18n._zh_family(tag)` (which shipped Chinese family a tag points to) is a private
//!   helper with no public Rust equivalent; the family it computes IS observable through
//!   `detect_locale`, so the family tests assert on `detect_locale`.
//! - Python "the negotiated chain always ends in `en`" (a missing translation surfaces the
//!   English SOURCE, never another language) -> a catalog miss in any `Locale` returns the
//!   input string from `text`, structurally (every shipped row already carries both zh columns).
//! - Python `i18n.gettext(msg)` -> `text(locale, msg)`.
//! - Python `i18n.gettext(tmpl) % {..}` (call-site %-substitution) -> `format_text(locale,
//!   tmpl, &[..])` ({}-substitution). Rust does not do %(name)s substitution.
//! - Python `i18n.available_locales()` -> `available_locale_tags()`.
//! - Python x-pseudo transform -> `text(Locale::Pseudo, ..)`.
//!
//! Buckets (each ignored test carries its reason inline and in the structured gap report):
//! - REAL (API exists): catalog completeness, `detect_locale` family/precedence rules, the
//!   x-pseudo transform, the English-source fallback guarantee, and the `nplurals=1` rule
//!   (encoded as identical zh strings on the singular/plural catalog pair).
//! - DIVERGENCE (full asserting body, `#[ignore]`): three verified gaps where Rust output
//!   contradicts the oracle — `zh-MY` and `zh-XX` resolve to English in Rust but Simplified in
//!   the oracle, and the term "Library" is translated `程序库`/`程式庫` in Rust but `工具库`/
//!   `工具庫` in the oracle `.po` (task #9, "Sweep the catalog back to the v0.4 translations").
//! - CROSS-CRATE (`#[ignore]` stub): env-var precedence lives in `skit-cli::cli::active_locale`
//!   and language persistence / corrupt-config recovery in `skit-store::config`; neither is
//!   reachable from this crate's integration tests.
//! - ABSENT (`#[ignore]` stub): `ngettext` selection-by-n, and the Babel-era source-extraction /
//!   unwrapped-string / dynamic-gettext scanners, which the compiled-catalog model does not have.

use std::borrow::Cow;

use skit_i18n::{
    Locale, available_locale_tags, catalog, detect_locale, format_text, requested_locale, text,
};

// =====================================================================================
// TestCatalogParity
// =====================================================================================

#[test]
fn test_locales_shipped() {
    // The .pot (English source) is the source of truth; skit must ship English plus at least
    // two translated locales.
    let tags = available_locale_tags();
    assert!(tags.contains(&"en"));
    assert!(tags.len() >= 3, "shipped locales: {tags:?}");
}

#[test]
fn test_parity_with_pot() {
    // Oracle: every shipped .po covers exactly the .pot msgid set — no missing translations,
    // no orphans. In the compiled catalog the "no orphans" half is discharged by the type: one
    // `Translation` row carries english + zh_cn + zh_tw together, so a zh string can never key a
    // msgid the source lacks. The "no missing" half is that every row's zh columns are present.
    let rows = catalog();
    assert!(!rows.is_empty(), "the catalog must not be empty");
    for row in rows {
        assert!(
            !row.zh_cn.trim().is_empty(),
            "zh-CN is missing a translation: {}",
            row.english
        );
        assert!(
            !row.zh_tw.trim().is_empty(),
            "zh-TW is missing a translation: {}",
            row.english
        );
    }
}

#[test]
#[ignore = "ABSENT (build-tooling scanner): the oracle extracts every gettext() msgid from src/skit with Babel and asserts freshness against skit.pot (test_i18n.py:45). The compiled static catalog has no separate source-extraction step to drift from — the catalog IS the source of truth — so this freshness check is structurally moot and has no Rust equivalent to call."]
fn test_pot_covers_all_source_gettext_msgids() {
    // Python: extract_from_dir(src/skit) msgids ⊆ _msgids(POT). No analog: skit-i18n has no
    // extraction pipeline; a user-visible string that is not a catalog row simply falls back to
    // its own English text at runtime.
}

#[test]
fn test_locale_is_fully_translated() {
    // Oracle completeness gate: parity only proves a locale CONTAINS every msgid; it can still
    // ship an empty or fuzzy msgstr (which renders as English). Every msgid in every shipped
    // locale must carry a non-empty translation. The compiled catalog has no "fuzzy" concept
    // (rows are reviewed static literals), so completeness reduces to "no empty zh string".
    let rows = catalog();
    assert!(!rows.is_empty(), "the catalog must not be empty");
    let mut untranslated: Vec<&str> = Vec::new();
    for row in rows {
        if row.zh_cn.trim().is_empty() || row.zh_tw.trim().is_empty() {
            untranslated.push(row.english);
        }
    }
    assert!(
        untranslated.is_empty(),
        "untranslated msgids: {untranslated:?}"
    );
}

#[test]
#[ignore = "CROSS-CRATE (frontend coverage): the oracle runs scripts/i18n_coverage.py scan_unwrapped over src/skit to catch UI literals (Static/Label/.notify/help=/…) not wrapped in gettext (test_i18n.py:101). The Rust analog would scan skit-tui/skit-cli sinks against the catalog; it is not reachable from this crate and there is no such scanner in the Rust surface."]
fn test_no_unwrapped_ui_strings() {
    // Python: mod.scan_unwrapped(src/skit) is empty. No analog here — the check belongs to the
    // frontend tier (skit-tui/skit-cli), not the skit-i18n catalog crate.
}

#[test]
#[ignore = "ABSENT (Babel-invisibility scanner): the oracle runs scan_dynamic_gettext to reject gettext(variable)/gettext(dict[k]) calls that Babel cannot extract (test_i18n.py:123). There is no Babel extraction in the compiled-catalog model, so a dynamic `text(kind)` is not invisible — it just falls back to English — and this detector has no Rust equivalent to call."]
fn test_no_dynamic_gettext() {
    // Python: scan_dynamic_gettext(src/skit) is empty AND fires on a gettext(dict[k]) sample.
    // No analog: extraction does not exist here, so the invisibility class it guards cannot occur.
}

// =====================================================================================
// TestNegotiation  (Python negotiate()[0] -> Rust detect_locale())
// =====================================================================================

#[test]
fn test_exact_match() {
    assert_eq!(detect_locale(Some("zh-TW")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh-TW")).tag(), "zh-TW");
}

#[test]
fn test_alias_hant() {
    // zh-HK / zh-Hant both map to Traditional Chinese.
    assert_eq!(detect_locale(Some("zh-HK")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh-Hant")), Locale::ZhTw);
}

#[test]
fn test_alias_hans_and_bare_zh() {
    assert_eq!(detect_locale(Some("zh")), Locale::ZhCn);
    assert_eq!(detect_locale(Some("zh-SG")), Locale::ZhCn);
}

#[test]
fn test_posix_style_tag() {
    // A POSIX spelling with a charset suffix resolves the same as the hyphen tag.
    assert_eq!(detect_locale(Some("zh_TW.UTF-8")), Locale::ZhTw);
}

#[test]
fn test_unknown_falls_back_to_en() {
    // An unshipped language family resolves to English.
    assert_eq!(detect_locale(Some("ko-KR")), Locale::En);
}

#[test]
fn test_chain_always_ends_with_en() {
    // Oracle: every negotiated chain ends in `en`, i.e. an untranslated msgid always surfaces
    // the English SOURCE. The compiled catalog has no chain list, but the same guarantee holds:
    // for the resolved `Locale` of any tag, an uncatalogued msgid returns its own English text.
    for tag in ["zh-TW", "zh", "ja", ""] {
        let locale = detect_locale(Some(tag));
        assert_eq!(
            text(locale, "no-such-message"),
            "no-such-message",
            "tag {tag:?}"
        );
    }
}

#[test]
fn test_traditional_chain_excludes_simplified() {
    // Regression: a Traditional tag must never smuggle Simplified in ahead of English. Every
    // Traditional-family tag resolves to zh-TW, and a msgid zh-TW does not cover falls straight
    // through to the English source — never a Simplified glyph.
    for tag in ["zh-TW", "zh-HK", "zh-MO", "zh-Hant"] {
        assert_eq!(detect_locale(Some(tag)), Locale::ZhTw, "tag {tag:?}");
    }
    let missing = text(Locale::ZhTw, "no-such-message");
    assert_eq!(missing, "no-such-message");
}

#[test]
fn test_simplified_chain_still_resolves_to_zh_cn() {
    // Preserve existing correct behavior: Simplified-family tags (and the bare "zh" request) all
    // resolve to zh-CN.
    for tag in ["zh-CN", "zh-Hans", "zh-SG", "zh-MY", "zh"] {
        assert_eq!(detect_locale(Some(tag)), Locale::ZhCn, "tag {tag:?}");
    }
}

#[test]
fn test_zh_region_with_no_script_hint_defaults_to_simplified() {
    // A "zh-*" tag whose subtag is not a known Hant/Hans hint has no inferable family, so the
    // bare-"zh" fallback step keeps its unconditional default of zh-CN. (_zh_family is a private
    // helper with no public Rust equivalent; only the negotiated primary is observable here.)
    assert_eq!(detect_locale(Some("zh-XX")), Locale::ZhCn);
}

#[test]
fn test_conflicting_script_and_region_lets_script_win() {
    // Regression: BCP-47 treats script as more specific than region, so an explicit script subtag
    // decides the family even when the region subtag belongs to the other family.
    // (_zh_family(tag) maps to detect_locale(tag), which is the resolved family.)
    assert_eq!(detect_locale(Some("zh-Hant-CN")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh-Hans-TW")), Locale::ZhCn);
}

#[test]
fn test_conflicting_script_and_region_hk_mo_variants() {
    // Same precedence rule over the other Traditional-region hints (hk, mo) paired with an
    // explicit Hans script subtag: script wins, so both resolve to zh-CN.
    for tag in ["zh-Hans-HK", "zh-Hans-MO"] {
        assert_eq!(detect_locale(Some(tag)), Locale::ZhCn, "tag {tag:?}");
    }
}

#[test]
fn test_plain_tags_unaffected_by_script_over_region_precedence() {
    // Regression: the already-correct non-conflicting cases (script and region agree, or only one
    // is present) are unchanged by the script-over-region rule.
    for tag in ["zh-TW", "zh-HK", "zh-Hant"] {
        assert_eq!(detect_locale(Some(tag)), Locale::ZhTw, "tag {tag:?}");
    }
    for tag in ["zh-CN", "zh-Hans", "zh"] {
        assert_eq!(detect_locale(Some(tag)), Locale::ZhCn, "tag {tag:?}");
    }
}

// =====================================================================================
// TestFormatting
// =====================================================================================

#[test]
fn test_zh_tw_message() {
    assert_eq!(text(Locale::ZhTw, "Name"), "名稱");
}

#[test]
fn test_zh_cn_message() {
    assert_eq!(text(Locale::ZhCn, "Name"), "名称");
}

#[test]
fn test_entry_library_copy_does_not_narrow_the_mixed_library_to_scripts() {
    // The generic library term must stay the mixed-library word (工具库/工具庫), never the
    // script-library word (脚本库/腳本庫), across every "Library"-bearing message.
    for (locale, library, script_library) in [
        (Locale::ZhCn, "工具库", "脚本库"),
        (Locale::ZhTw, "工具庫", "腳本庫"),
    ] {
        let messages = [
            text(locale, "Library").into_owned(),
            format_text(locale, "Library: {} ({} · {})", &[&"/lib", &"3", &"1 KB"]),
            text(locale, "Return to the Library immediately").into_owned(),
            text(locale, "Description (shown in the Library)").into_owned(),
        ];
        assert!(
            messages.iter().all(|message| message.contains(library)),
            "{locale:?}: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .all(|message| !message.contains(script_library)),
            "{locale:?}: {messages:?}"
        );
    }
}

#[test]
#[ignore = "ABSENT (no ngettext): the oracle selects a plural form by n via i18n.ngettext (test_i18n.py:290). skit-i18n has no selection-by-n API — English plurals are distinct catalog rows (\"{} entry registered\" vs \"{} entries registered\") chosen by the caller (skit-cli/skit-tui), so there is nothing in this crate to call."]
fn test_en_plural() {
    // Python: ngettext(sing, plur, 1).endswith("entry"); ngettext(sing, plur, 5).endswith("entries").
}

#[test]
fn test_zh_plural_single_form() {
    // Chinese has one plural form (nplurals=1): singular and plural render identically. In the
    // static catalog this is encoded as data — the zh strings on the singular/plural row pair are
    // byte-identical — so the same behavioral claim holds without an ngettext call.
    assert_eq!(
        text(Locale::ZhCn, "{} entry registered"),
        text(Locale::ZhCn, "{} entries registered")
    );
    assert_eq!(
        text(Locale::ZhTw, "{} entry registered"),
        text(Locale::ZhTw, "{} entries registered")
    );
}

#[test]
fn test_variable_substitution() {
    // A translated template plus a call-site value keeps the value verbatim. (Rust substitutes a
    // {} hole via format_text rather than Python's %(file)s; the sentence is the identity source,
    // so text() returns it unchanged and the value lands in the hole.)
    let message = format_text(
        Locale::En,
        "{} isn't a .py file — pass --exe if it's an executable",
        &[&"photo.py"],
    );
    assert!(message.contains("photo.py"), "{message}");
}

#[test]
fn test_missing_id_returns_source() {
    assert_eq!(text(Locale::En, "no-such-message"), "no-such-message");
}

#[test]
fn test_pseudo_locale() {
    let value = text(Locale::Pseudo, "Name");
    assert!(value.starts_with('⟦'), "{value}");
    assert!(value.ends_with('⟧'), "{value}");
    assert!(
        value.contains('à') || value.to_lowercase().contains('é') || value.contains("Nàmé"),
        "{value}"
    );
}

#[test]
fn test_pseudo_preserves_placeholder() {
    // %-placeholders must survive the pseudo transform so call-site substitution still works.
    // (Rust preserves the %(name)s run intact; it does not perform the % substitution itself.)
    let value = text(
        Locale::Pseudo,
        "%(file)s isn't a .py file — pass --exe if it's an executable",
    );
    assert!(value.contains("%(file)s"), "{value}");
}

// =====================================================================================
// TestScriptAwareFallback
//
// The oracle monkeypatches i18n._LOCALES_DIR with synthetic .mo catalogs so a genuinely
// missing translation can be exercised. The compiled catalog cannot be swapped for a fake dir,
// so these assert the same GUARANTEE structurally: a request for one family never returns the
// other family's glyphs; a catalogued row returns that family's translation, an uncatalogued one
// returns the English source.
// =====================================================================================

#[test]
fn test_missing_zh_tw_msgid_falls_back_to_english_not_simplified() {
    // zh-TW returns its Traditional translation for a catalogued row, and the English source for
    // an uncatalogued one — NOT the Simplified translation.
    assert_eq!(text(Locale::ZhTw, "Name"), "名稱");
    let missing = text(Locale::ZhTw, "New String");
    assert_eq!(missing, "New String");
    assert_ne!(missing, "新字符串");
}

#[test]
fn test_missing_zh_hk_msgid_falls_back_to_english_not_simplified() {
    // zh-HK aliases to the zh-TW family; the same guarantee applies.
    assert_eq!(detect_locale(Some("zh-HK")), Locale::ZhTw);
    assert_eq!(text(Locale::ZhTw, "New String"), "New String");
}

#[test]
fn test_zh_cn_still_gets_its_own_translation() {
    // Unaffected: Simplified resolution to zh-CN still returns its own translation. (The oracle's
    // synthetic "New String" -> 新字符串 is not in the compiled catalog; "Name" -> 名称 is the
    // real-catalog stand-in for the same claim.)
    assert_eq!(text(Locale::ZhCn, "Name"), "名称");
}

#[test]
fn test_conflicting_script_and_region_tag_end_to_end() {
    // Regression, mirror direction: zh-Hans-HK carries an explicit Simplified script subtag over a
    // Traditional-associated "HK" region. Script wins, so it resolves to zh-CN and renders the
    // Simplified glyph, never the Traditional one.
    assert_eq!(detect_locale(Some("zh-Hans-HK")), Locale::ZhCn);
    assert_eq!(text(Locale::ZhCn, "Name"), "名称");
    assert_ne!(text(Locale::ZhCn, "Name"), Cow::Borrowed("名稱"));
}

// =====================================================================================
// TestDetection  (env-var precedence lives in skit-cli::cli::active_locale)
// =====================================================================================

#[test]
#[ignore = "CROSS-CRATE (composition root): SKIT_LANG > config.toml > LC_ALL > LC_MESSAGES > LANG precedence is wired in skit-cli::cli::active_locale (crates/skit-cli/src/cli.rs:218). skit-i18n's requested_locale resolves ONE candidate; it does not read env vars, so the override-precedence assertion is not reachable from this crate."]
fn test_env_override_wins() {
    // Python: SKIT_LANG=zh-TW beats LANG=en_US.UTF-8 -> detect_locale() == "zh-TW".
}

#[test]
#[ignore = "CROSS-CRATE (composition root): the LANG-derived resolution runs through skit-cli::cli::active_locale (crates/skit-cli/src/cli.rs:231). skit-i18n does not consult env vars, so this belongs to the CLI tier's tests."]
fn test_lang_env() {
    // Python: LANG=zh_CN.UTF-8 (with SKIT_LANG/LC_ALL/LC_MESSAGES unset) -> detect_locale() == "zh-CN".
}

#[test]
fn test_c_locale_ignored() {
    // The exact POSIX "C" locale is not a language preference: requested_locale returns None so
    // the precedence chain skips it and tries the next source. A tag that merely contains C, such
    // as "C.UTF-8", is a real (English) preference and stops the chain.
    assert_eq!(requested_locale(Some("C")), None);
    assert_eq!(requested_locale(Some("c")), None);
    assert_eq!(requested_locale(Some("C.UTF-8")), Some(Locale::En));
}

#[test]
#[ignore = "CROSS-CRATE (store): set_language writes config.toml and _config_language reads it back (test_i18n.py:418). Persistence lives in skit-store::config::FileConfigStore (crates/skit-store/src/config.rs), driven from skit-cli; skit-i18n has no config-writing surface."]
fn test_set_language_persists() {
    // Python: set_language("zh-TW") writes config.toml, returns "zh-TW", and _config_language()
    // reads it back; set_language("") clears it.
}

// =====================================================================================
// TestSetLanguageCorruptConfig  (corrupt config.toml recovery — skit-store)
// =====================================================================================

#[test]
#[ignore = "CROSS-CRATE (store recovery): set_language uses atomic.load_toml_recoverable so a present-but-corrupt config.toml is backed up to config.toml.bak (with a stderr warning) instead of being wiped (test_i18n.py:437). This recovery path lives in skit-store::config/atomic, not in skit-i18n."]
fn test_backs_up_corrupt_config_instead_of_wiping_it() {
    // Python: a corrupt config.toml is preserved verbatim in config.toml.bak; the requested
    // language change still takes effect; the user is warned on stderr (both paths named).
}

#[test]
#[ignore = "CROSS-CRATE (store recovery): the branch where the corrupt config cannot even be backed up (copy2 raises) still applies the change and warns on stderr (test_i18n.py:461). Owned by skit-store::config/atomic; not reachable from skit-i18n."]
fn test_warns_when_corrupt_config_cannot_even_be_backed_up() {
    // Python: when the backup copy fails, no .bak is created, the change still lands, and stderr
    // still names config.toml.
}

#[test]
#[ignore = "CROSS-CRATE (store): a valid, parseable config must NOT trigger the recovery path (no .bak) and the [mirror] section must survive a language change (test_i18n.py:480). Owned by skit-store::config; skit-i18n has no config surface."]
fn test_valid_config_is_unaffected() {
    // Python: a valid config keeps its [mirror] table after set_language("en") and no .bak appears.
}
