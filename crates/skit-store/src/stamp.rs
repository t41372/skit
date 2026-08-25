//! Spell the machine timestamps version 0.4 stores.

use time::{OffsetDateTime, UtcOffset};

/// Spell one instant the way version 0.4 spells a stored timestamp.
///
/// Version 0.4 stamps every stored time with
/// `datetime.now(UTC).replace(microsecond=0).isoformat()` (`models.py:248`), which drops the
/// fractional second and names the offset `+00:00`. The default RFC 3339 rendering keeps
/// nanoseconds and names that same offset `Z`, so a stamp this product wrote would not match the
/// stamp version 0.4 wrote for the same instant. Read paths stay permissive: they accept both
/// spellings, so a library written by either version reads back unchanged.
///
/// The instant moves to UTC first, because the offset in the result is the literal `+00:00`.
#[must_use]
pub fn iso_stamp(timestamp: OffsetDateTime) -> String {
    let timestamp = timestamp.to_offset(UtcOffset::UTC);
    let date = timestamp.date();
    let time = timestamp.time();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        date.year(),
        u8::from(date.month()),
        date.day(),
        time.hour(),
        time.minute(),
        time.second(),
    )
}

/// Stamp this instant the way version 0.4 stamps it.
#[must_use]
pub fn now_iso() -> String {
    iso_stamp(OffsetDateTime::now_utc())
}

#[cfg(test)]
mod tests {
    use time::{Date, Month, Time};

    use super::*;

    /// The exact text version 0.4 writes: whole seconds, and the offset named `+00:00`.
    ///
    /// Version 0.4 produced `2026-08-23T09:48:54+00:00` for this instant when its own `now_iso`
    /// ran. Every part is pinned, so a stamp that drops the truncation, renames the offset, or
    /// reorders a field is a different string.
    #[test]
    fn a_stamp_spells_whole_seconds_and_names_the_offset_the_way_version_0_4_names_it() {
        let date = Date::from_calendar_date(2026, Month::August, 23).unwrap();
        let time = Time::from_hms_nano(9, 48, 54, 654_992_369).unwrap();
        let timestamp = date.with_time(time).assume_utc();

        assert_eq!(iso_stamp(timestamp), "2026-08-23T09:48:54+00:00");
    }

    /// Each field keeps its own place and its leading zero.
    #[test]
    fn a_stamp_pads_every_field_that_version_0_4_pads() {
        let date = Date::from_calendar_date(2026, Month::January, 2).unwrap();
        let time = Time::from_hms(3, 4, 5).unwrap();

        assert_eq!(
            iso_stamp(date.with_time(time).assume_utc()),
            "2026-01-02T03:04:05+00:00"
        );
    }

    /// The offset in the text is a literal, so the instant moves to UTC before it is spelled.
    #[test]
    fn a_stamp_reads_the_same_instant_from_another_offset() {
        let date = Date::from_calendar_date(2026, Month::August, 23).unwrap();
        let time = Time::from_hms(11, 48, 54).unwrap();
        let elsewhere = date
            .with_time(time)
            .assume_offset(UtcOffset::from_hms(2, 0, 0).unwrap());

        assert_eq!(iso_stamp(elsewhere), "2026-08-23T09:48:54+00:00");
    }

    /// The current stamp has the shape every stored stamp has.
    #[test]
    fn the_current_stamp_has_the_stored_shape() {
        let stamp = now_iso();

        assert_eq!(stamp.len(), 25, "{stamp}");
        assert!(stamp.ends_with("+00:00"), "{stamp}");
        assert!(!stamp.contains('.'), "{stamp}");
        assert_eq!(&stamp[10..11], "T", "{stamp}");
    }
}
