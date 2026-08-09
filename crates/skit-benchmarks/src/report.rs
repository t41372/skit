//! Result summaries, headline rendering, and run-directory validation.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    Meta, PipelineError, Results, ResultsError, SuiteKind, SuiteOutput,
    budget::{Budget, BudgetReport, evaluate, format_number, render_report},
    merge,
};

/// Stable latest-main headline set.
pub const HEADLINE_METRICS: &[&str] = &[
    "startup.version.median_ms",
    "startup.version.over_python_ms",
    "startup.list_json.median_ms",
    "scale.list_json.n100.median_ms",
    "scale.list_json.n1000.median_ms",
    "scale.list_json.per_entry_us",
    "run_overhead.python.overhead_ms",
    "run_overhead.shell.overhead_ms",
    "tui.first_idle.n100.median_ms",
    "tui.first_idle.n1000.median_ms",
    "tui.select.n1000.median_ms",
    "tui.search.n1000.median_ms",
    "rss.version.peak_kib",
    "rss.list_json.n1000.peak_kib",
    "imports.version.modules",
    "imports.list_json.n0.modules",
    "imports.list_json.n100.modules",
    "imports.list_json.n100.has_tree_sitter",
    "footprint.wheel_bytes",
    "footprint.closure_bytes",
    "footprint.library_total_bytes.n1000",
    "footprint.library_bytes_per_entry.n1000",
    "micro.store.list_entries.n1000.median_us",
    "micro.store.list_summaries.n1000.median_us",
    "syscalls.list_json.file_ops",
    "pipeline.duration_s",
];

/// Run provenance and completion stamp written after every planned suite succeeds.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RunRecord {
    /// Environment manifest.
    pub meta: Meta,
    /// End-to-end duration.
    pub total_duration_s: f64,
    /// Planned suite set. Old schema-v1 run files can omit this.
    #[serde(default)]
    pub suites: Vec<SuiteKind>,
}

/// A run directory could not be summarized safely.
#[derive(Debug, Error)]
pub enum SummaryError {
    /// A required file is absent.
    #[error("{0}")]
    Missing(String),
    /// Run JSON failed.
    #[error("run.json is not valid: {0}")]
    RunJson(#[from] serde_json::Error),
    /// Suite or result validation failed.
    #[error(transparent)]
    Results(#[from] ResultsError),
    /// Merge failed.
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    /// Filesystem operation failed.
    #[error("could not {operation} {path}: {source}")]
    Io {
        /// Operation.
        operation: &'static str,
        /// Path.
        path: PathBuf,
        /// OS error.
        #[source]
        source: std::io::Error,
    },
    /// Run and suite files disagree.
    #[error(
        "run directory suite set does not match run.json: expected {expected:?}, found {found:?}"
    )]
    SuiteSet {
        /// Planned set.
        expected: Vec<SuiteKind>,
        /// Files found.
        found: Vec<SuiteKind>,
    },
    /// A result input was a symlink or another unexpected filesystem object.
    #[error("refusing to summarize unexpected filesystem object {0}")]
    UnsafePath(PathBuf),
}

/// Merge a completed run directory and atomically publish JSON and Markdown.
pub fn summarize_directory(
    bench_dir: &Path,
    budgets: Option<&[Budget]>,
) -> Result<Results, SummaryError> {
    let run_path = bench_dir.join("run.json");
    if !normal_file_exists(&run_path)? {
        return Err(SummaryError::Missing(format!(
            "no run.json in {} - did the run complete?",
            bench_dir.display()
        )));
    }
    let run_text = read(&run_path)?;
    let run: RunRecord = serde_json::from_str(&run_text)?;
    if !run.total_duration_s.is_finite() || run.total_duration_s < 0.0 {
        return Err(SummaryError::Missing(
            "run.json total_duration_s must be finite and non-negative".to_owned(),
        ));
    }
    let suites_dir = bench_dir.join("suites");
    ensure_normal_directory(&suites_dir)?;
    let reader = fs::read_dir(&suites_dir).map_err(|source| SummaryError::Io {
        operation: "scan",
        path: suites_dir.clone(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in reader {
        let path = entry
            .map_err(|source| SummaryError::Io {
                operation: "scan",
                path: suites_dir.clone(),
                source,
            })?
            .path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            if !normal_file_exists(&path)? {
                return Err(SummaryError::UnsafePath(path));
            }
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(SummaryError::Missing(format!(
            "no suite outputs under {}",
            suites_dir.display()
        )));
    }
    let mut outputs = Vec::with_capacity(paths.len());
    for path in paths {
        outputs.push(SuiteOutput::from_json(&read(&path)?)?);
    }
    let mut found = outputs
        .iter()
        .map(|output| output.suite)
        .collect::<Vec<_>>();
    found.sort();
    if !run.suites.is_empty() {
        let mut expected = run.suites.clone();
        expected.sort();
        if expected != found {
            return Err(SummaryError::SuiteSet { expected, found });
        }
    }
    let results = merge(run.meta, outputs, run.total_duration_s)?;
    let report = budgets.map(|budgets| evaluate(budgets, &results));
    atomic_write(&bench_dir.join("results.json"), &results.to_json()?)?;
    atomic_write(
        &bench_dir.join("results.md"),
        &render_results_markdown(&results, report.as_ref()),
    )?;
    Ok(results)
}

/// Render one compact artifact summary.
#[must_use]
pub fn render_results_markdown(results: &Results, report: Option<&BudgetReport>) -> String {
    let meta = &results.meta;
    let dirty = if meta.git.dirty { " (dirty)" } else { "" };
    let mut lines = vec![
        "## Benchmark results".to_owned(),
        String::new(),
        format!(
            "skit {} @ `{}`{dirty} · profile **{}** · {}",
            meta.skit_version,
            meta.git.commit.get(..12).unwrap_or(&meta.git.commit),
            meta.profile.as_str(),
            meta.generated_at
        ),
        String::new(),
        format!(
            "{} {} · {} x {} · {} MiB · python {} · uv {} · textual {} · pyperf {} · runner {}{}",
            meta.host.os,
            meta.host.kernel,
            meta.host.cpu,
            meta.host.cpu_count,
            meta.host.mem_total_mib,
            meta.python,
            meta.uv,
            meta.textual,
            meta.pyperf,
            meta.host.ci_runner.as_deref().unwrap_or("local"),
            meta.host
                .ci_image_version
                .as_ref()
                .map_or_else(String::new, |version| format!(" image {version}"))
        ),
        String::new(),
        "| Metric | Value | p95 | n |".to_owned(),
        "| --- | ---: | ---: | ---: |".to_owned(),
    ];
    for metric_id in HEADLINE_METRICS {
        let Some(metric) = results.metrics.get(*metric_id) else {
            continue;
        };
        lines.push(format!(
            "| `{metric_id}` | {} {} | {} | {} |",
            format_number(metric.value),
            metric.unit,
            metric.p95.map_or_else(|| "—".to_owned(), format_number),
            metric.n
        ));
    }
    if results.skipped.is_empty() {
        lines.extend([String::new(), "No skipped cases.".to_owned()]);
    } else {
        lines.extend([
            String::new(),
            format!("### Skipped ({})", results.skipped.len()),
            String::new(),
        ]);
        lines.extend(
            results
                .skipped
                .iter()
                .map(|skip| format!("- `{}/{}`: {}", skip.suite.as_str(), skip.case, skip.reason)),
        );
    }
    if let Some(report) = report {
        lines.extend([
            String::new(),
            "### Budgets".to_owned(),
            String::new(),
            "```".to_owned(),
            render_report(report).trim_end().to_owned(),
            "```".to_owned(),
        ]);
    }
    lines.push(String::new());
    lines.join("\n")
}

fn read(path: &Path) -> Result<String, SummaryError> {
    fs::read_to_string(path).map_err(|source| SummaryError::Io {
        operation: "read",
        path: path.to_path_buf(),
        source,
    })
}

fn ensure_normal_directory(path: &Path) -> Result<(), SummaryError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| SummaryError::Io {
        operation: "inspect",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(SummaryError::UnsafePath(path.to_path_buf()))
    }
}

fn normal_file_exists(path: &Path) -> Result<bool, SummaryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(SummaryError::UnsafePath(path.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(SummaryError::Io {
            operation: "inspect",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn atomic_write(path: &Path, text: &str) -> Result<(), SummaryError> {
    let parent = path.parent().ok_or_else(|| {
        SummaryError::Missing(format!("{} has no parent directory", path.display()))
    })?;
    let mut staged =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| SummaryError::Io {
            operation: "create",
            path: parent.to_path_buf(),
            source,
        })?;
    staged
        .write_all(text.as_bytes())
        .map_err(|source| SummaryError::Io {
            operation: "write",
            path: staged.path().to_path_buf(),
            source,
        })?;
    staged.persist(path).map_err(|error| SummaryError::Io {
        operation: "commit",
        path: path.to_path_buf(),
        source: error.error,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::TempDir;

    #[test]
    fn filesystem_helpers_keep_each_failure_typed() {
        let root = TempDir::new().unwrap();
        let missing = root.path().join("missing");
        assert!(matches!(super::normal_file_exists(&missing), Ok(false)));
        assert!(matches!(
            super::read(&missing),
            Err(super::SummaryError::Io {
                operation: "read",
                ..
            })
        ));
        assert!(matches!(
            super::ensure_normal_directory(&missing),
            Err(super::SummaryError::Io {
                operation: "inspect",
                ..
            })
        ));
        assert!(matches!(
            super::normal_file_exists(root.path()),
            Err(super::SummaryError::UnsafePath(_))
        ));
        assert!(matches!(
            super::atomic_write(Path::new("/"), "value"),
            Err(super::SummaryError::Missing(_))
        ));

        let absent_parent = root.path().join("absent/output");
        assert!(matches!(
            super::atomic_write(&absent_parent, "value"),
            Err(super::SummaryError::Io {
                operation: "create",
                ..
            })
        ));

        let directory = root.path().join("directory");
        fs::create_dir(&directory).unwrap();
        assert!(matches!(
            super::normal_file_exists(&directory),
            Err(super::SummaryError::UnsafePath(_))
        ));
    }
}
