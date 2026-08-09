//! Pure parsers for operating-system benchmark tools.

use std::{collections::BTreeMap, sync::LazyLock};

use regex::Regex;
use thiserror::Error;

/// Tool output did not match the promised format.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ParseError {
    /// Named measurement was absent.
    #[error("{0}")]
    Missing(&'static str),
    /// A numeric field was malformed.
    #[error("{0}")]
    Invalid(String),
}

/// Parse Linux `/proc/*/status` peak RSS in KiB.
pub fn vmhwm_kib(text: &str) -> Result<u64, ParseError> {
    let line = text
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .ok_or(ParseError::Missing("no VmHWM line in status text"))?;
    line.split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ParseError::Invalid("invalid VmHWM value".to_owned()))
}

/// Parse GNU time's `%M` output in KiB.
pub fn gnu_time_max_kib(text: &str) -> Result<u64, ParseError> {
    text.trim()
        .parse()
        .map_err(|_| ParseError::Invalid("invalid GNU time maximum RSS".to_owned()))
}

/// Parse BSD time's maximum-resident-set-size line and normalize bytes to KiB.
pub fn bsd_time_max_kib(text: &str) -> Result<u64, ParseError> {
    for line in text.lines() {
        if !line.contains("maximum resident set size") {
            continue;
        }
        let bytes = line
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| ParseError::Invalid("invalid BSD time maximum RSS".to_owned()))?;
        return Ok(bytes / 1_024);
    }
    Err(ParseError::Missing(
        "no maximum resident set size in BSD time output",
    ))
}

static STRACE_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*[\d.]+\s+[\d.]+\s+\d+\s+(\d+)\s+(?:\d+\s+)?([a-z0-9_]+)\s*$")
        .expect("fixed strace regex")
});

/// Parse `strace -c` into calls per syscall.
pub fn strace_counts(text: &str) -> Result<BTreeMap<String, u64>, ParseError> {
    let mut counts = BTreeMap::new();
    for line in text.lines() {
        let Some(captures) = STRACE_ROW.captures(line) else {
            continue;
        };
        let name = captures[2].to_owned();
        if name == "total" {
            continue;
        }
        let calls = captures[1]
            .parse::<u64>()
            .expect("the strace regular expression captures decimal digits");
        *counts.entry(name).or_default() += calls;
    }
    if counts.is_empty() {
        Err(ParseError::Missing(
            "no syscall rows found in strace -c output",
        ))
    } else {
        Ok(counts)
    }
}

/// Count a named syscall group.
#[must_use]
pub fn count_group(counts: &BTreeMap<String, u64>, names: &[&str]) -> u64 {
    names.iter().filter_map(|name| counts.get(*name)).sum()
}

/// File operations used by the read-path contract.
pub const FILE_OP_SYSCALLS: &[&str] = &[
    "open",
    "openat",
    "openat2",
    "stat",
    "lstat",
    "fstat",
    "newfstatat",
    "statx",
    "read",
];
/// Network operations expected to stay at zero.
pub const NETWORK_SYSCALLS: &[&str] = &["socket", "connect"];

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        FILE_OP_SYSCALLS, NETWORK_SYSCALLS, ParseError, bsd_time_max_kib, count_group,
        gnu_time_max_kib, strace_counts, vmhwm_kib,
    };

    #[test]
    fn peak_memory_parsers_accept_native_tool_formats() {
        assert_eq!(
            vmhwm_kib("Name:\tskit\nVmHWM:\t  12345 kB\n").unwrap(),
            12_345
        );
        assert_eq!(gnu_time_max_kib(" 54321\n").unwrap(), 54_321);
        assert_eq!(
            bsd_time_max_kib("  2097152  maximum resident set size\n").unwrap(),
            2_048
        );
    }

    #[test]
    fn peak_memory_parsers_reject_missing_and_malformed_values() {
        assert_eq!(
            vmhwm_kib("VmRSS: 12 kB\n"),
            Err(ParseError::Missing("no VmHWM line in status text"))
        );
        assert_eq!(
            vmhwm_kib("VmHWM: nope kB\n"),
            Err(ParseError::Invalid("invalid VmHWM value".to_owned()))
        );
        assert_eq!(
            gnu_time_max_kib("nope"),
            Err(ParseError::Invalid(
                "invalid GNU time maximum RSS".to_owned()
            ))
        );
        assert_eq!(
            bsd_time_max_kib("no maximum here\n"),
            Err(ParseError::Missing(
                "no maximum resident set size in BSD time output"
            ))
        );
        assert_eq!(
            bsd_time_max_kib("nope maximum resident set size\n"),
            Err(ParseError::Invalid(
                "invalid BSD time maximum RSS".to_owned()
            ))
        );
    }

    #[test]
    fn strace_parser_keeps_calls_with_and_without_error_columns() {
        let counts = strace_counts(
            "% time seconds usecs/call calls errors syscall\n\
             40.00 0.004 4 7 openat\n\
             30.00 0.003 3 2 1 openat\n\
             20.00 0.002 2 3 socket\n\
             100.00 0.010 10 12 1 total\n",
        )
        .unwrap();
        assert_eq!(counts["openat"], 9);
        assert_eq!(counts["socket"], 3);
        assert_eq!(count_group(&counts, FILE_OP_SYSCALLS), 9);
        assert_eq!(count_group(&counts, NETWORK_SYSCALLS), 3);
        assert_eq!(count_group(&BTreeMap::new(), FILE_OP_SYSCALLS), 0);
    }

    #[test]
    fn strace_parser_rejects_output_without_rows() {
        assert_eq!(
            strace_counts("% time seconds usecs/call calls syscall\n---\n"),
            Err(ParseError::Missing(
                "no syscall rows found in strace -c output"
            ))
        );
    }
}
