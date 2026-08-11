use proptest::prelude::*;
use skit_domain::{DomainError, EntryId, EntryKind, EntryMeta, EntrySummary, Slug, StorageMode};

#[test]
fn slugify_matches_the_existing_address_rules() {
    assert_eq!(
        Slug::from_display_name("  Hello,   世界!  ").as_str(),
        "hello-世界"
    );
    assert_eq!(Slug::from_display_name("---").as_str(), "script");
    assert_eq!(Slug::from_display_name("A___B---C").as_str(), "a-b-c");
}

#[test]
fn parsed_slugs_reject_non_canonical_addresses() {
    for value in ["", "-leading", "trailing-", "two--dashes", "Upper"] {
        assert!(Slug::parse(value).is_err(), "{value:?} should be refused");
    }
    let slug = Slug::parse("hello-世界").unwrap();
    assert_eq!(slug.as_str(), "hello-世界");
    assert_eq!(slug.to_string(), "hello-世界");
}

#[test]
fn slugs_round_trip_through_frontend_json() {
    let slug = Slug::parse("hello-world").unwrap();
    let json = serde_json::to_string(&slug).unwrap();
    assert_eq!(json, "\"hello-world\"");
    assert_eq!(serde_json::from_str::<Slug>(&json).unwrap(), slug);
    assert!(serde_json::from_str::<Slug>("\"Upper\"").is_err());
}

#[test]
fn kinds_are_open_for_forward_compatibility() {
    let kind = EntryKind::parse(" language-added-by-a-newer-skit ").unwrap();
    assert_eq!(kind.as_str(), "language-added-by-a-newer-skit");
    assert_eq!(kind.to_string(), "language-added-by-a-newer-skit");
    assert!(EntryKind::parse("  ").is_err());

    let json = serde_json::to_string(&kind).unwrap();
    assert_eq!(serde_json::from_str::<EntryKind>(&json).unwrap(), kind);
    assert!(serde_json::from_str::<EntryKind>("\" \"").is_err());
}

#[test]
fn entry_ids_accept_and_normalize_uuid_formats() {
    let simple = EntryId::parse("0123456789abcdef0123456789abcdef").unwrap();
    let hyphenated = EntryId::parse("01234567-89ab-cdef-0123-456789abcdef").unwrap();
    assert_eq!(simple, hyphenated);
    assert_eq!(simple.as_str(), "0123456789abcdef0123456789abcdef");
    assert_eq!(simple.to_string(), simple.as_str());
    assert!(EntryId::parse("").is_err());
    assert!(EntryId::parse("not-an-id").is_err());

    let json = serde_json::to_string(&simple).unwrap();
    assert_eq!(serde_json::from_str::<EntryId>(&json).unwrap(), simple);
    assert!(serde_json::from_str::<EntryId>("\"not-an-id\"").is_err());
}

#[test]
fn generated_entry_ids_are_valid_and_unique_enough_for_identity_claims() {
    let first = EntryId::generate();
    let second = EntryId::generate();
    assert_eq!(first.as_str().len(), 32);
    assert_ne!(first, second);
    assert_eq!(EntryId::parse(first.as_str()).unwrap(), first);
}

#[test]
fn storage_mode_uses_the_existing_toml_spelling() {
    assert_eq!(
        serde_json::to_string(&StorageMode::Copy).unwrap(),
        "\"copy\""
    );
    assert_eq!(
        serde_json::from_str::<StorageMode>("\"reference\"").unwrap(),
        StorageMode::Reference
    );
    assert_eq!(StorageMode::default(), StorageMode::Copy);
}

#[test]
fn minimal_metadata_has_legacy_safe_defaults() {
    let meta = EntryMeta::minimal("Hello", EntryKind::parse("python").unwrap());
    assert_eq!(meta.schema, 1);
    assert_eq!(meta.name, "Hello");
    assert_eq!(meta.mode, StorageMode::Copy);
    assert_eq!(meta.workdir, "origin");
    assert!(meta.id.is_none());
    assert!(meta.source.is_empty());
    assert!(meta.source_hash.is_empty());
    assert!(meta.added_at.is_empty());
    assert!(meta.description.is_empty());
    assert!(meta.extra.is_empty());
}

#[test]
fn entry_projections_round_trip_without_losing_open_kinds() {
    let summary = EntrySummary {
        slug: Slug::parse("hello").unwrap(),
        name: "Hello".to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode: StorageMode::Reference,
        description: "description".to_owned(),
        target: Some("/tmp/hello".to_owned()),
    };
    let json = serde_json::to_string(&summary).unwrap();
    assert_eq!(
        serde_json::from_str::<EntrySummary>(&json).unwrap(),
        summary
    );
}

#[test]
fn domain_errors_are_specific_and_displayable() {
    assert_eq!(
        DomainError::InvalidSlug("Bad".to_owned()).to_string(),
        "invalid entry slug: Bad"
    );
    assert_eq!(
        DomainError::InvalidKind.to_string(),
        "entry kind cannot be blank"
    );
    assert_eq!(
        DomainError::InvalidEntryId("bad".to_owned()).to_string(),
        "invalid entry id: bad"
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
