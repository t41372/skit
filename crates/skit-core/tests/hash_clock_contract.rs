use std::time::{Duration, UNIX_EPOCH};

use skit_core::{format_utc_timestamp, sha256_source_hash};

#[test]
fn sha256_matches_standard_vectors() {
    assert_eq!(
        sha256_source_hash(b""),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_source_hash(b"abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_source_hash(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "sha256:248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn sha256_handles_multiple_blocks() {
    let data = vec![b'a'; 1_000_000];
    assert_eq!(
        sha256_source_hash(&data),
        "sha256:cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn utc_timestamp_matches_python_metadata_spelling() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        format_utc_timestamp(UNIX_EPOCH)?,
        "1970-01-01T00:00:00+00:00"
    );
    let sample = UNIX_EPOCH + Duration::from_secs(1_786_161_906);
    assert_eq!(format_utc_timestamp(sample)?, "2026-08-08T04:05:06+00:00");
    Ok(())
}

#[test]
fn utc_timestamp_handles_leap_day_and_year_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let leap_day = UNIX_EPOCH + Duration::from_secs(1_709_251_199);
    assert_eq!(format_utc_timestamp(leap_day)?, "2024-02-29T23:59:59+00:00");
    let new_year = UNIX_EPOCH + Duration::from_secs(1_735_689_600);
    assert_eq!(format_utc_timestamp(new_year)?, "2025-01-01T00:00:00+00:00");
    Ok(())
}
