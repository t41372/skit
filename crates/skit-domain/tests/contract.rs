use proptest::prelude::*;
use skit_domain::{EntryId, EntryKind, Slug, StorageMode};

#[test]
fn slugify_matches_the_existing_address_rules() {
    assert_eq!(Slug::from_display_name("  Hello,   世界!  ").as_str(), "hello-世界");
    assert_eq!(Slug::from_display_name("---").as_str(), "script");
    assert_eq!(Slug::from_display_name("A___B---C").as_str(), "a-b-c");
}

#[test]
fn parsed_slugs_reject_non_canonical_addresses() {
    for value in ["", "-leading", "trailing-", "two--dashes", "Upper"] {
        assert!(Slug::parse(value).is_err(), "{value:?} should be refused");
    }
    assert_eq!(Slug::parse("hello-世界").unwrap().as_str(), "hello-世界");
}

#[test]
fn kinds_are_open_for_forward_compatibility() {
    let kind = EntryKind::parse("language-added-by-a-newer-skit").unwrap();
    assert_eq!(kind.as_str(), "language-added-by-a-newer-skit");
    assert!(EntryKind::parse("  ").is_err());
}

#[test]
fn entry_ids_accept_the_current_uuid_hex_format() {
    let id = EntryId::parse("0123456789abcdef0123456789abcdef").unwrap();
    assert_eq!(id.as_str(), "0123456789abcdef0123456789abcdef");
    assert!(EntryId::parse("").is_err());
    assert!(EntryId::parse("not-an-id").is_err());
}

#[test]
fn storage_mode_uses_the_existing_toml_spelling() {
    assert_eq!(serde_json::to_string(&StorageMode::Copy).unwrap(), "\"copy\"");
    assert_eq!(
        serde_json::from_str::<StorageMode>("\"reference\"").unwrap(),
        StorageMode::Reference
    );
}

proptest! {
    #[test]
    fn slugification_is_idempotent(input in ".{0,128}") {
        let once = Slug::from_display_name(&input);
        let twice = Slug::from_display_name(once.as_str());
        prop_assert_eq!(once, twice);
    }
}
