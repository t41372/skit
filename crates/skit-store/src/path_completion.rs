//! Read-only filesystem adapter for path completion.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use skit_application::path_completion::{
    DirectoryEntry, DirectoryReadError, DirectoryReadFilter, DirectoryReader,
};

/// System directory reader with no cache and no writes.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemDirectoryReader;

impl DirectoryReader for SystemDirectoryReader {
    fn read_directory(
        &self,
        path: &Path,
        scan_cap: usize,
        filter: &DirectoryReadFilter,
    ) -> Result<Vec<DirectoryEntry>, DirectoryReadError> {
        let entries = fs::read_dir(path).map_err(|_| DirectoryReadError::Unavailable)?;
        collect_entries(
            entries.map(|item| {
                item.map(|entry| (entry.file_name(), entry.path()))
                    .map_err(|_| DirectoryReadError::Unavailable)
            }),
            scan_cap,
            filter,
            |path| fs::metadata(path).ok().map(|metadata| metadata.is_dir()),
        )
    }
}

fn collect_entries(
    entries: impl IntoIterator<Item = Result<(OsString, PathBuf), DirectoryReadError>>,
    scan_cap: usize,
    filter: &DirectoryReadFilter,
    mut directory_status: impl FnMut(&Path) -> Option<bool>,
) -> Result<Vec<DirectoryEntry>, DirectoryReadError> {
    let mut result = Vec::new();
    for item in entries.into_iter().take(scan_cap) {
        let (name, path) = item?;
        let Ok(name) = name.into_string() else {
            continue;
        };
        if !filter.accepts(&name) {
            continue;
        }
        result.push(if directory_status(&path).unwrap_or(false) {
            DirectoryEntry::directory(name)
        } else {
            DirectoryEntry::file(name)
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> Result<(OsString, PathBuf), DirectoryReadError> {
        Ok((OsString::from(name), PathBuf::from(name)))
    }

    #[test]
    fn scan_cap_counts_examined_entries_before_any_later_filter() {
        let entries = collect_entries(
            [row("one"), row("two"), row("three"), row("four")],
            3,
            &DirectoryReadFilter::new("", true),
            |_| Some(false),
        )
        .unwrap();
        assert_eq!(
            entries
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            ["one", "two", "three"]
        );
    }

    #[test]
    fn iteration_failure_discards_partial_scan() {
        assert_eq!(
            collect_entries(
                [row("one"), Err(DirectoryReadError::Unavailable)],
                3,
                &DirectoryReadFilter::new("", true),
                |_| Some(false),
            ),
            Err(DirectoryReadError::Unavailable)
        );
    }

    #[test]
    fn metadata_failure_degrades_one_entry_to_a_file() {
        assert_eq!(
            collect_entries(
                [row("gone")],
                3,
                &DirectoryReadFilter::new("", true),
                |_| None,
            )
            .unwrap(),
            [DirectoryEntry::file("gone")]
        );
    }

    #[test]
    fn prefix_and_hidden_misses_count_toward_the_cap_without_metadata_probes() {
        let mut probed = Vec::new();
        let entries = collect_entries(
            [
                row(".private"),
                row("other"),
                row("prefix"),
                row("preferred-after-cap"),
            ],
            3,
            &DirectoryReadFilter::new("pre", false),
            |path| {
                probed.push(path.to_path_buf());
                Some(true)
            },
        )
        .unwrap();

        assert_eq!(entries, [DirectoryEntry::directory("prefix")]);
        assert_eq!(probed, [PathBuf::from("prefix")]);
    }
}
