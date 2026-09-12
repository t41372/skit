//! Hyperfine command construction and export parsing.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use thiserror::Error;

use crate::{
    Metric,
    stats::{StatsError, median, nearest_rank_p95, sample_stddev},
};

/// Hyperfine version installed by the repository action.
pub const HYPERFINE_VERSION: &str = "1.20.0";
/// SHA-256 for the pinned Linux archive.
pub const HYPERFINE_SHA256: &str =
    "63ad53934062118f5b0be11785e0bb1603d4b91667d1921f2fd8df9a8712040a";
/// Upstream archive installed by CI.
pub const HYPERFINE_URL: &str = "https://github.com/sharkdp/hyperfine/releases/download/v1.20.0/hyperfine-v1.20.0-x86_64-unknown-linux-gnu.tar.gz";

/// One command measured by Hyperfine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Case {
    /// Stable metric prefix.
    pub name: String,
    /// Exact target argv.
    pub argv: Vec<String>,
}

impl Case {
    /// Build one case without using a shell command string as the source of truth.
    pub fn new<I, S>(name: impl Into<String>, argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            name: name.into(),
            argv: argv.into_iter().map(Into::into).collect(),
        }
    }
}

/// Hyperfine input or output did not match the harness contract.
#[derive(Debug, Error)]
pub enum HyperfineError {
    /// A batch had no commands.
    #[error("no cases to benchmark")]
    EmptyCases,
    /// A target argv was empty.
    #[error("hyperfine case {0:?} has an empty argv")]
    EmptyArgv(String),
    /// A stable case identifier was empty.
    #[error("hyperfine case name is empty")]
    EmptyName,
    /// A stable case identifier occurred more than once.
    #[error("duplicate hyperfine case name {0:?}")]
    DuplicateCase(String),
    /// An argv value cannot be represented in Hyperfine's shell-none command field.
    #[error("cannot quote hyperfine case {case:?}: {reason}")]
    Quote {
        /// Case name.
        case: String,
        /// Quote error.
        reason: String,
    },
    /// The export is invalid JSON.
    #[error("hyperfine export is not JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The export has an invalid shape or sample.
    #[error("{0}")]
    Shape(String),
    /// Statistical summarization failed.
    #[error(transparent)]
    Stats(#[from] StatsError),
}

/// Build the exact Hyperfine argv used by macro suites.
pub fn build_argv(
    cases: &[Case],
    warmup: usize,
    minimum_runs: usize,
    export_json: &str,
    hyperfine: &str,
) -> Result<Vec<String>, HyperfineError> {
    if cases.is_empty() {
        return Err(HyperfineError::EmptyCases);
    }
    let mut argv = vec![
        hyperfine.to_owned(),
        "--shell=none".to_owned(),
        "--style".to_owned(),
        "basic".to_owned(),
        "--warmup".to_owned(),
        warmup.to_string(),
        "--min-runs".to_owned(),
        minimum_runs.to_string(),
        "--export-json".to_owned(),
        export_json.to_owned(),
    ];
    let mut names = BTreeSet::new();
    for case in cases {
        if case.name.is_empty() {
            return Err(HyperfineError::EmptyName);
        }
        if !names.insert(case.name.as_str()) {
            return Err(HyperfineError::DuplicateCase(case.name.clone()));
        }
        if case.argv.is_empty() {
            return Err(HyperfineError::EmptyArgv(case.name.clone()));
        }
        let command = shlex::try_join(case.argv.iter().map(String::as_str)).map_err(|error| {
            HyperfineError::Quote {
                case: case.name.clone(),
                reason: error.to_string(),
            }
        })?;
        argv.extend(["--command-name".to_owned(), case.name.clone(), command]);
    }
    Ok(argv)
}

#[derive(Debug, Deserialize)]
struct Export {
    results: Vec<ExportResult>,
}

#[derive(Debug, Deserialize)]
struct ExportResult {
    command: String,
    times: Vec<f64>,
    #[serde(default)]
    exit_codes: Vec<i32>,
}

/// Parse full times in seconds keyed by case name.
pub fn parse_export(text: &str) -> Result<BTreeMap<String, Vec<f64>>, HyperfineError> {
    let export: Export = serde_json::from_str(text)?;
    if export.results.is_empty() {
        return Err(HyperfineError::Shape(
            "hyperfine export has no results".to_owned(),
        ));
    }
    let mut output = BTreeMap::new();
    for result in export.results {
        if result.command.is_empty() || result.times.is_empty() {
            return Err(HyperfineError::Shape(
                "hyperfine result entry missing command/times".to_owned(),
            ));
        }
        if result
            .times
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(HyperfineError::Shape(format!(
                "hyperfine case {:?}: time must be finite and non-negative",
                result.command
            )));
        }
        if result.exit_codes.iter().any(|code| *code != 0) {
            return Err(HyperfineError::Shape(format!(
                "hyperfine case {:?} recorded non-zero exit codes",
                result.command
            )));
        }
        if output
            .insert(result.command.clone(), result.times)
            .is_some()
        {
            return Err(HyperfineError::Shape(format!(
                "duplicate hyperfine case {:?}",
                result.command
            )));
        }
    }
    Ok(output)
}

/// Require the export to contain exactly the requested stable case identifiers.
pub fn validate_case_names(
    samples: &BTreeMap<String, Vec<f64>>,
    cases: &[Case],
) -> Result<(), HyperfineError> {
    let expected = cases
        .iter()
        .map(|case| case.name.as_str())
        .collect::<BTreeSet<_>>();
    let actual = samples.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if expected == actual {
        Ok(())
    } else {
        Err(HyperfineError::Shape(format!(
            "hyperfine export case set differs: expected {expected:?}, found {actual:?}"
        )))
    }
}

/// Convert samples in seconds to the standard millisecond metric.
pub fn metric_from_times(times_seconds: &[f64]) -> Result<Metric, HyperfineError> {
    let milliseconds = times_seconds
        .iter()
        .map(|seconds| seconds * 1_000.0)
        .collect::<Vec<_>>();
    Ok(Metric {
        value: median(&milliseconds)?,
        unit: "ms".to_owned(),
        n: milliseconds.len(),
        p95: Some(nearest_rank_p95(&milliseconds)?),
        stddev: Some(sample_stddev(&milliseconds)?),
    })
}

/// Mint each `<case>.median_ms` metric from an export.
pub fn metrics_from_export(
    samples: &BTreeMap<String, Vec<f64>>,
) -> Result<BTreeMap<String, Metric>, HyperfineError> {
    samples
        .iter()
        .map(|(name, values)| Ok((format!("{name}.median_ms"), metric_from_times(values)?)))
        .collect()
}
