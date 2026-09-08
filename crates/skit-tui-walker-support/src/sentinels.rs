use std::collections::BTreeMap;

use serde_json::{Value, json};

/// Stable ranks for values first observed in ascending order.
///
/// The allocator assigns each rank once and never reassigns it. The table never
/// shrinks. A deletion does not renumber live values, so their ranks can be sparse.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AscendingRankAllocator {
    ranks: BTreeMap<u64, u64>,
}

impl AscendingRankAllocator {
    /// Assign the next rank to each new value in an ascending scan.
    ///
    /// Each new value must exceed all values from earlier scans and all values
    /// assigned earlier in this scan. The scan must be ascending and contain
    /// unique values. Duplicate detection compares adjacent values, so the
    /// ascending precondition is also required to find every duplicate.
    pub fn assign_sorted(&mut self, ascending: &[u64]) -> Result<(), String> {
        if let Some(duplicate) = ascending.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(format!(
                "rank scan contains duplicate value {}",
                duplicate[0]
            ));
        }

        let mut ranks = self.ranks.clone();
        for observed in ascending {
            if ranks.contains_key(observed) {
                continue;
            }
            if let Some(maximum) = ranks.last_key_value().map(|(value, _)| *value)
                && *observed <= maximum
            {
                return Err(format!(
                    "rank value {observed} does not exceed the current maximum {maximum}"
                ));
            }
            let rank = ranks.len() as u64;
            ranks.insert(*observed, rank);
        }
        self.ranks = ranks;
        Ok(())
    }

    /// Return the assigned rank for one observed value.
    #[must_use]
    pub fn lookup(&self, observed: u64) -> Option<u64> {
        self.ranks.get(&observed).copied()
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SameFileProjection {
    Unix {
        device: u64,
        inode: u64,
    },
    Windows {
        volume_serial_number: u64,
        file_index: u128,
        creation_time: u64,
    },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct UnixIdentityChange {
    seconds: i64,
    nanoseconds: i64,
}

/// Typed source-identity sentinels that preserve same-file comparisons.
#[derive(Debug, Default)]
pub struct SourceIdentitySentinels {
    files: BTreeMap<SameFileProjection, u64>,
    changes: BTreeMap<SameFileProjection, BTreeMap<UnixIdentityChange, u64>>,
}

impl SourceIdentitySentinels {
    /// Construct a canonical typed sentinel for one serialized source identity.
    pub fn construct(&mut self, observed: &Value) -> Result<Value, String> {
        let platform = observed
            .get("platform")
            .ok_or_else(|| "source identity has no platform tag".to_owned())?
            .as_str()
            .ok_or_else(|| "source identity platform tag is not text".to_owned())?;
        match platform {
            "unix" => self.construct_unix(observed),
            "windows" => self.construct_windows(observed),
            unknown => Err(format!("source identity platform is unknown: {unknown}")),
        }
    }

    fn construct_unix(&mut self, observed: &Value) -> Result<Value, String> {
        let projection = SameFileProjection::Unix {
            device: u64_field(observed, "device")?,
            inode: u64_field(observed, "inode")?,
        };
        let change = UnixIdentityChange {
            seconds: i64_field(observed, "change_time_seconds")?,
            nanoseconds: i64_field(observed, "change_time_nanoseconds")?,
        };
        let identity = dense_rank(&mut self.files, projection.clone());
        let change = dense_rank(self.changes.entry(projection).or_default(), change);
        Ok(json!({
            "platform": "unix",
            "device": 0,
            "inode": identity,
            "change_time_seconds": 0,
            "change_time_nanoseconds": change,
        }))
    }

    fn construct_windows(&mut self, observed: &Value) -> Result<Value, String> {
        let projection = SameFileProjection::Windows {
            volume_serial_number: u64_field(observed, "volume_serial_number")?,
            file_index: decimal_u128_field(observed, "file_index")?,
            creation_time: u64_field(observed, "creation_time")?,
        };
        let identity = dense_rank(&mut self.files, projection);
        Ok(json!({
            "platform": "windows",
            "volume_serial_number": 0,
            "file_index": identity.to_string(),
            // The projection already carries the creation time.
            "creation_time": 0,
        }))
    }
}

fn dense_rank<Key: Ord>(ranks: &mut BTreeMap<Key, u64>, key: Key) -> u64 {
    let next = ranks.len() as u64;
    *ranks.entry(key).or_insert(next)
}

fn u64_field(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("source identity field {field} is not a u64"))
}

fn i64_field(value: &Value, field: &str) -> Result<i64, String> {
    value
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("source identity field {field} is not an i64"))
}

fn decimal_u128_field(value: &Value, field: &str) -> Result<u128, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .and_then(|encoded| encoded.parse().ok())
        .ok_or_else(|| format!("source identity field {field} is not a decimal u128"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AscendingRankAllocator, SourceIdentitySentinels};

    #[test]
    fn ascending_rank_allocator_assigns_dense_stable_ranks() {
        let mut ranks = AscendingRankAllocator::default();

        assert_eq!(ranks.lookup(10), None);
        assert_eq!(ranks.assign_sorted(&[]), Ok(()));
        assert_eq!(ranks.assign_sorted(&[10, 20]), Ok(()));
        assert_eq!(ranks.lookup(10), Some(0));
        assert_eq!(ranks.lookup(20), Some(1));
        assert_eq!(ranks.assign_sorted(&[10, 20, 30]), Ok(()));
        assert_eq!(ranks.lookup(10), Some(0));
        assert_eq!(ranks.lookup(20), Some(1));
        assert_eq!(ranks.lookup(30), Some(2));
    }

    #[test]
    fn ascending_rank_allocator_refuses_duplicate_values_atomically() {
        let mut ranks = AscendingRankAllocator::default();
        ranks.assign_sorted(&[5]).unwrap();
        let before = ranks.clone();

        assert_eq!(
            ranks.assign_sorted(&[10, 10]),
            Err("rank scan contains duplicate value 10".to_owned())
        );
        assert_eq!(ranks, before);
        assert_eq!(ranks.lookup(10), None);
        assert_eq!(ranks.assign_sorted(&[5]), Ok(()));
    }

    #[test]
    fn ascending_rank_allocator_refuses_a_new_value_below_its_maximum() {
        let mut ranks = AscendingRankAllocator::default();
        ranks.assign_sorted(&[10, 30]).unwrap();

        assert_eq!(
            ranks.assign_sorted(&[20]),
            Err("rank value 20 does not exceed the current maximum 30".to_owned())
        );
        assert_eq!(ranks.lookup(20), None);
        assert_eq!(ranks.lookup(30), Some(1));
    }

    #[test]
    fn source_identity_sentinels_keep_unix_same_file_and_change_generations() {
        let mut sentinels = SourceIdentitySentinels::default();
        let first = json!({
            "platform": "unix",
            "device": 7,
            "inode": 8,
            "change_time_seconds": 9,
            "change_time_nanoseconds": 10,
        });
        assert_eq!(
            sentinels.construct(&first).unwrap(),
            json!({
                "platform": "unix",
                "device": 0,
                "inode": 0,
                "change_time_seconds": 0,
                "change_time_nanoseconds": 0,
            })
        );
        assert_eq!(
            sentinels.construct(&first).unwrap(),
            sentinels.construct(&first).unwrap()
        );

        let changed_nanoseconds = json!({
            "platform": "unix",
            "device": 7,
            "inode": 8,
            "change_time_seconds": 9,
            "change_time_nanoseconds": 11,
        });
        assert_eq!(
            sentinels.construct(&changed_nanoseconds).unwrap(),
            json!({
                "platform": "unix",
                "device": 0,
                "inode": 0,
                "change_time_seconds": 0,
                "change_time_nanoseconds": 1,
            })
        );

        let changed_seconds = json!({
            "platform": "unix",
            "device": 7,
            "inode": 8,
            "change_time_seconds": 10,
            "change_time_nanoseconds": 0,
        });
        assert_eq!(
            sentinels.construct(&changed_seconds).unwrap(),
            json!({
                "platform": "unix",
                "device": 0,
                "inode": 0,
                "change_time_seconds": 0,
                "change_time_nanoseconds": 2,
            })
        );

        let different_file = json!({
            "platform": "unix",
            "device": 7,
            "inode": 9,
            "change_time_seconds": 9,
            "change_time_nanoseconds": 10,
        });
        assert_eq!(
            sentinels.construct(&different_file).unwrap(),
            json!({
                "platform": "unix",
                "device": 0,
                "inode": 1,
                "change_time_seconds": 0,
                "change_time_nanoseconds": 0,
            })
        );
    }

    #[test]
    fn source_identity_sentinels_emit_windows_file_indexes_as_decimal_strings() {
        let mut sentinels = SourceIdentitySentinels::default();
        sentinels
            .construct(&json!({
                "platform": "unix",
                "device": 1,
                "inode": 2,
                "change_time_seconds": 3,
                "change_time_nanoseconds": 4,
            }))
            .unwrap();
        let unix_change_count = sentinels.changes.len();
        let windows = json!({
            "platform": "windows",
            "volume_serial_number": 5,
            "file_index": "340282366920938463463374607431768211455",
            "creation_time": 6,
        });
        assert_eq!(
            sentinels.construct(&windows).unwrap(),
            json!({
                "platform": "windows",
                "volume_serial_number": 0,
                "file_index": "1",
                "creation_time": 0,
            })
        );
        assert_eq!(sentinels.construct(&windows).unwrap()["file_index"], "1");
        assert_eq!(sentinels.changes.len(), unix_change_count);

        let different_creation = json!({
            "platform": "windows",
            "volume_serial_number": 5,
            "file_index": "340282366920938463463374607431768211455",
            "creation_time": 7,
        });
        assert_eq!(
            sentinels.construct(&different_creation).unwrap(),
            json!({
                "platform": "windows",
                "volume_serial_number": 0,
                "file_index": "2",
                "creation_time": 0,
            })
        );
    }

    #[test]
    fn source_identity_sentinels_refuse_invalid_platforms_and_fields() {
        let mut sentinels = SourceIdentitySentinels::default();

        assert_eq!(
            sentinels.construct(&json!({})),
            Err("source identity has no platform tag".to_owned())
        );
        assert_eq!(
            sentinels.construct(&json!({"platform": 1})),
            Err("source identity platform tag is not text".to_owned())
        );
        assert_eq!(
            sentinels.construct(&json!({"platform": "plan9"})),
            Err("source identity platform is unknown: plan9".to_owned())
        );
        assert_eq!(
            sentinels.construct(&json!({
                "platform": "unix",
                "device": "7",
                "inode": 8,
                "change_time_seconds": 9,
                "change_time_nanoseconds": 10,
            })),
            Err("source identity field device is not a u64".to_owned())
        );
        assert_eq!(
            sentinels.construct(&json!({
                "platform": "unix",
                "device": 7,
                "inode": 8,
                "change_time_seconds": "9",
                "change_time_nanoseconds": 10,
            })),
            Err("source identity field change_time_seconds is not an i64".to_owned())
        );
        assert_eq!(
            sentinels.construct(&json!({
                "platform": "windows",
                "volume_serial_number": "5",
                "file_index": "6",
                "creation_time": 7,
            })),
            Err("source identity field volume_serial_number is not a u64".to_owned())
        );
        assert_eq!(
            sentinels.construct(&json!({
                "platform": "windows",
                "volume_serial_number": 5,
                "file_index": "not-decimal",
                "creation_time": 7,
            })),
            Err("source identity field file_index is not a decimal u128".to_owned())
        );
    }
}
