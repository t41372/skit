//! Typed performance-evaluation plans for skit.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod budget;
pub mod compare;
pub mod dataset;
pub mod environment;
pub mod hyperfine;
pub mod parsers;
pub mod pipeline;
pub mod process;
mod python_random;
pub mod report;
pub mod runner;
pub mod sources;
pub mod stats;
pub mod suites;
pub mod tui_probe;

pub use budget::BudgetOutcome;

/// A reproducible benchmark profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkProfile {
    /// Pull-request measurements.
    Pr,
    /// Complete nightly measurements.
    Full,
    /// One side of an A/B comparison.
    Compare,
}

impl BenchmarkProfile {
    /// Stable command-line and artifact identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pr => "pr",
            Self::Full => "full",
            Self::Compare => "compare",
        }
    }
}

impl FromStr for BenchmarkProfile {
    type Err = ProfileError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pr" => Ok(Self::Pr),
            "full" => Ok(Self::Full),
            "compare" => Ok(Self::Compare),
            _ => Err(ProfileError(value.to_owned())),
        }
    }
}

/// A command requested an unknown benchmark profile.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("unknown profile {0:?} (expected pr, full, or compare)")]
pub struct ProfileError(String);

/// One benchmark suite.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuiteKind {
    /// Loaded-module census.
    Imports,
    /// Wheel, archive, dependency, and library sizes.
    Footprint,
    /// Peak resident memory.
    Rss,
    /// Process startup latency.
    Startup,
    /// Library-size scaling.
    Scale,
    /// Launcher overhead relative to each language runtime.
    RunOverhead,
    /// In-process hot-path measurements.
    Micro,
    /// Headless terminal interaction latency.
    Tui,
    /// Read-path system-call census.
    Syscalls,
}

impl SuiteKind {
    /// Stable machine identifier for the suite.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Imports => "imports",
            Self::Footprint => "footprint",
            Self::Rss => "rss",
            Self::Startup => "startup",
            Self::Scale => "scale",
            Self::RunOverhead => "run_overhead",
            Self::Micro => "micro",
            Self::Tui => "tui",
            Self::Syscalls => "syscalls",
        }
    }
}

/// Parameters for one suite execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SuitePlan {
    /// Suite implementation to run.
    pub kind: SuiteKind,
    /// Deterministic library sizes used by the suite.
    pub library_sizes: Vec<usize>,
    /// Warmup process count for timing tools.
    pub warmup: usize,
    /// Minimum measured process count.
    pub minimum_runs: usize,
    /// Independent samples for probes that do not use a timing harness.
    pub samples: usize,
    /// Use the fast microbenchmark configuration.
    pub fast: bool,
    /// Measure the complete installed dependency closure.
    pub measure_closure: bool,
    /// Measure the JavaScript run lane.
    pub run_javascript_lane: bool,
    /// Include the doctor scale case.
    pub run_doctor: bool,
    /// Degrade incompatible per-case imports to recorded skips for A/B runs.
    pub compare_mode: bool,
}

impl SuitePlan {
    fn new(kind: SuiteKind) -> Self {
        Self {
            kind,
            library_sizes: Vec::new(),
            warmup: 3,
            minimum_runs: 15,
            samples: 5,
            fast: true,
            measure_closure: false,
            run_javascript_lane: false,
            run_doctor: false,
            compare_mode: false,
        }
    }

    fn sizes(mut self, sizes: &[usize]) -> Self {
        self.library_sizes = sizes.to_vec();
        self
    }

    const fn timing(mut self, warmup: usize, minimum_runs: usize) -> Self {
        self.warmup = warmup;
        self.minimum_runs = minimum_runs;
        self
    }

    const fn samples(mut self, samples: usize) -> Self {
        self.samples = samples;
        self
    }

    const fn closure(mut self) -> Self {
        self.measure_closure = true;
        self
    }

    const fn javascript(mut self) -> Self {
        self.run_javascript_lane = true;
        self
    }

    const fn doctor(mut self) -> Self {
        self.run_doctor = true;
        self
    }

    const fn rigorous(mut self) -> Self {
        self.fast = false;
        self
    }

    const fn compare(mut self) -> Self {
        self.compare_mode = true;
        self
    }
}

/// Build the suite table for one profile.
#[must_use]
pub fn build_plan(profile: BenchmarkProfile) -> Vec<SuitePlan> {
    match profile {
        BenchmarkProfile::Pr => pr_plan(),
        BenchmarkProfile::Full => full_plan(),
        BenchmarkProfile::Compare => pr_plan()
            .into_iter()
            .filter(|suite| suite.kind != SuiteKind::Footprint)
            .map(SuitePlan::compare)
            .collect(),
    }
}

fn pr_plan() -> Vec<SuitePlan> {
    vec![
        SuitePlan::new(SuiteKind::Imports).sizes(&[0, 100]),
        SuitePlan::new(SuiteKind::Footprint).sizes(&[0, 1_000]),
        SuitePlan::new(SuiteKind::Rss).sizes(&[0, 1_000]).samples(5),
        SuitePlan::new(SuiteKind::Startup).sizes(&[0]).timing(3, 15),
        SuitePlan::new(SuiteKind::Scale)
            .sizes(&[0, 100, 1_000])
            .timing(3, 15),
        SuitePlan::new(SuiteKind::RunOverhead).timing(3, 15),
        SuitePlan::new(SuiteKind::Micro).sizes(&[0, 100, 1_000]),
        SuitePlan::new(SuiteKind::Tui)
            .sizes(&[0, 100, 1_000])
            .samples(5),
    ]
}

fn full_plan() -> Vec<SuitePlan> {
    vec![
        SuitePlan::new(SuiteKind::Imports).sizes(&[0, 100]),
        SuitePlan::new(SuiteKind::Footprint)
            .sizes(&[0, 1_000])
            .closure(),
        SuitePlan::new(SuiteKind::Rss)
            .sizes(&[0, 1_000])
            .samples(10),
        SuitePlan::new(SuiteKind::Startup).sizes(&[0]).timing(5, 40),
        SuitePlan::new(SuiteKind::Scale)
            .sizes(&[0, 10, 100, 1_000])
            .timing(5, 40)
            .doctor(),
        SuitePlan::new(SuiteKind::RunOverhead)
            .timing(5, 40)
            .javascript(),
        SuitePlan::new(SuiteKind::Micro)
            .sizes(&[0, 100, 1_000])
            .rigorous(),
        SuitePlan::new(SuiteKind::Tui)
            .sizes(&[0, 100, 1_000])
            .samples(10),
        SuitePlan::new(SuiteKind::Syscalls).sizes(&[1_000]),
    ]
}

/// Return the sorted union of library sizes required by a plan.
#[must_use]
pub fn dataset_sizes(plan: &[SuitePlan]) -> Vec<usize> {
    plan.iter()
        .flat_map(|suite| suite.library_sizes.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// One numeric benchmark result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Metric {
    /// Headline or median value.
    pub value: f64,
    /// Stable unit token.
    pub unit: String,
    /// Number of measured samples.
    #[serde(rename = "n")]
    pub n: usize,
    /// Nearest-rank 95th percentile when the suite records it.
    pub p95: Option<f64>,
    /// Population or sample standard deviation reported by the measurement tool.
    pub stddev: Option<f64>,
}

impl Metric {
    /// Construct one deterministic observation.
    #[must_use]
    pub fn single(value: f64, unit: &str) -> Self {
        Self {
            value,
            unit: unit.to_owned(),
            n: 1,
            p95: None,
            stddev: None,
        }
    }
}

/// Git identity for the measured checkout.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitInfo {
    /// Exact measured commit.
    pub commit: String,
    /// Whether tracked or untracked files changed the measured checkout.
    pub dirty: bool,
    /// Durable pull-request anchor for ephemeral merge commits.
    pub pr: Option<String>,
}

/// Host facts needed to interpret and compare a run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostInfo {
    /// Operating-system name.
    pub os: String,
    /// Kernel release.
    pub kernel: String,
    /// CPU model.
    pub cpu: String,
    /// Logical CPU count.
    pub cpu_count: usize,
    /// Total physical memory in MiB.
    pub mem_total_mib: usize,
    /// Stable OS and architecture key used by budget predicates.
    pub platform_key: String,
    /// CI runner label, or `None` for a local run.
    pub ci_runner: Option<String>,
    /// CI image version when the provider exposes one.
    #[serde(default)]
    pub ci_image_version: Option<String>,
}

/// Reproducibility manifest for one benchmark run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Meta {
    /// UTC generation timestamp.
    pub generated_at: String,
    /// Executed suite profile.
    pub profile: BenchmarkProfile,
    /// Measured Git checkout.
    pub git: GitInfo,
    /// Measured skit version.
    pub skit_version: String,
    /// Host facts.
    pub host: HostInfo,
    /// Python version used for compatibility baselines.
    pub python: String,
    /// uv version.
    pub uv: String,
    /// Textual version for compatibility with version 0.4 benchmark artifacts.
    pub textual: String,
    /// pyperf version for compatibility with version 0.4 benchmark artifacts.
    #[serde(default = "unknown_version")]
    pub pyperf: String,
}

/// One benchmark case that could not run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Skip {
    /// Suite that owns the case.
    pub suite: SuiteKind,
    /// Stable case identifier.
    pub case: String,
    /// Recorded reason. A missing tool is a skip, but a suite crash is not.
    pub reason: String,
}

/// Output from one suite runner.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SuiteOutput {
    /// Suite that produced the output.
    pub suite: SuiteKind,
    /// Suite wall time in seconds.
    #[serde(rename = "duration_s")]
    pub duration_seconds: f64,
    /// Stable metric identifiers and values.
    pub metrics: std::collections::BTreeMap<String, Metric>,
    /// Cases the suite could not measure.
    pub skipped: Vec<Skip>,
    /// Tool output needed to audit or reparse the measurement.
    pub raw: std::collections::BTreeMap<String, serde_json::Value>,
}

impl SuiteOutput {
    /// Record that a pre-spawn capability check excluded the complete suite.
    #[must_use]
    pub fn skip_all(suite: SuiteKind, reason: impl Into<String>) -> Self {
        Self {
            suite,
            duration_seconds: 0.0,
            metrics: std::collections::BTreeMap::new(),
            skipped: vec![Skip {
                suite,
                case: "all".to_owned(),
                reason: reason.into(),
            }],
            raw: std::collections::BTreeMap::new(),
        }
    }

    /// Serialize a stable suite artifact.
    pub fn to_json(&self) -> Result<String, ResultsError> {
        self.validate()?;
        stable_json(self)
    }

    /// Parse and validate a suite artifact.
    pub fn from_json(text: &str) -> Result<Self, ResultsError> {
        let output: Self = serde_json::from_str(text)?;
        output.validate()?;
        Ok(output)
    }

    fn validate(&self) -> Result<(), ResultsError> {
        if !self.duration_seconds.is_finite() || self.duration_seconds < 0.0 {
            return Err(invalid(
                "duration_seconds",
                "expected a finite non-negative number",
            ));
        }
        validate_metrics(&self.metrics)?;
        validate_skips(&self.skipped)?;
        if let Some((index, _)) = self
            .skipped
            .iter()
            .enumerate()
            .find(|(_, skip)| skip.suite != self.suite)
        {
            return Err(invalid(
                format!("skipped[{index}].suite"),
                "expected the suite output label",
            ));
        }
        Ok(())
    }
}

/// Merged result document before host metadata is attached.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Results {
    /// Stable artifact schema version.
    pub schema_version: u32,
    /// Reproducibility manifest.
    pub meta: Meta,
    /// Complete metric namespace.
    pub metrics: std::collections::BTreeMap<String, Metric>,
    /// Every recorded skip.
    pub skipped: Vec<Skip>,
    /// Raw output keyed by suite identifier.
    pub raw: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Backward-compatible name for the merged artifact.
pub type BenchmarkResults = Results;

impl Results {
    /// Serialize stable, pretty JSON with a final newline.
    pub fn to_json(&self) -> Result<String, ResultsError> {
        self.validate()?;
        stable_json(self)
    }

    /// Parse and validate a benchmark artifact.
    pub fn from_json(text: &str) -> Result<Self, ResultsError> {
        let results: Self = serde_json::from_str(text)?;
        results.validate()?;
        Ok(results)
    }

    fn validate(&self) -> Result<(), ResultsError> {
        if self.schema_version != 1 {
            return Err(ResultsError::SchemaVersion(self.schema_version));
        }
        self.meta.validate()?;
        validate_metrics(&self.metrics)?;
        validate_skips(&self.skipped)
    }
}

impl Meta {
    fn validate(&self) -> Result<(), ResultsError> {
        for (path, value) in [
            ("meta.generated_at", self.generated_at.as_str()),
            ("meta.git.commit", self.git.commit.as_str()),
            ("meta.skit_version", self.skit_version.as_str()),
            ("meta.host.os", self.host.os.as_str()),
            ("meta.host.kernel", self.host.kernel.as_str()),
            ("meta.host.cpu", self.host.cpu.as_str()),
            ("meta.host.platform_key", self.host.platform_key.as_str()),
            ("meta.python", self.python.as_str()),
            ("meta.uv", self.uv.as_str()),
            ("meta.textual", self.textual.as_str()),
            ("meta.pyperf", self.pyperf.as_str()),
        ] {
            if value.is_empty() {
                return Err(invalid(path, "expected a non-empty string"));
            }
        }
        if self.git.pr.as_deref() == Some("") {
            return Err(invalid(
                "meta.git.pr",
                "expected a non-empty string or null",
            ));
        }
        if self.host.cpu_count == 0 {
            return Err(invalid(
                "meta.host.cpu_count",
                "expected a positive integer",
            ));
        }
        Ok(())
    }
}

/// A result document failed structural or semantic validation.
#[derive(Debug, Error)]
pub enum ResultsError {
    /// JSON could not be parsed into the result schema.
    #[error("invalid benchmark JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The artifact uses an unsupported schema version.
    #[error("schema_version: expected 1, got {0}")]
    SchemaVersion(u32),
    /// A typed field failed an invariant that JSON cannot express.
    #[error("{path}: {reason}")]
    InvalidField {
        /// JSON-style field path.
        path: String,
        /// Expected invariant.
        reason: &'static str,
    },
}

fn validate_metrics(
    metrics: &std::collections::BTreeMap<String, Metric>,
) -> Result<(), ResultsError> {
    for (metric_id, metric) in metrics {
        let prefix = format!("metrics.{metric_id}");
        if metric_id.is_empty() {
            return Err(invalid("metrics", "metric identifiers must not be empty"));
        }
        if !metric.value.is_finite() {
            return Err(invalid(
                format!("{prefix}.value"),
                "expected a finite number",
            ));
        }
        if metric.unit.is_empty() {
            return Err(invalid(
                format!("{prefix}.unit"),
                "expected a non-empty string",
            ));
        }
        if metric.n == 0 {
            return Err(invalid(
                format!("{prefix}.n"),
                "expected a positive integer",
            ));
        }
        for (name, value) in [("p95", metric.p95), ("stddev", metric.stddev)] {
            if value.is_some_and(|value| !value.is_finite()) {
                return Err(invalid(
                    format!("{prefix}.{name}"),
                    "expected a finite number or null",
                ));
            }
        }
    }
    Ok(())
}

fn validate_skips(skipped: &[Skip]) -> Result<(), ResultsError> {
    for (index, skip) in skipped.iter().enumerate() {
        for (name, value) in [
            ("case", skip.case.as_str()),
            ("reason", skip.reason.as_str()),
        ] {
            if value.is_empty() {
                return Err(invalid(
                    format!("skipped[{index}].{name}"),
                    "expected a non-empty string",
                ));
            }
        }
    }
    Ok(())
}

fn invalid(path: impl Into<String>, reason: &'static str) -> ResultsError {
    ResultsError::InvalidField {
        path: path.into(),
        reason,
    }
}

fn unknown_version() -> String {
    "unknown".to_owned()
}

fn stable_json<T: Serialize>(value: &T) -> Result<String, ResultsError> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text)
}

/// A benchmark plan or merge contract is inconsistent.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PipelineError {
    /// Run metadata failed its result-schema invariants.
    #[error("invalid benchmark metadata: {0}")]
    InvalidMeta(String),
    /// A suite output failed its result-schema invariants.
    #[error("invalid suite output {suite:?}: {reason}")]
    InvalidSuiteOutput {
        /// Output suite label.
        suite: String,
        /// Failed invariant.
        reason: String,
    },
    /// Total run time was negative or non-finite.
    #[error("total benchmark duration must be finite and non-negative")]
    InvalidTotalDuration,
    /// Derived output failed a result-schema invariant.
    #[error("merged benchmark results are invalid: {0}")]
    InvalidMergedResults(String),
    /// A suite tried to publish a reserved pipeline metric.
    #[error("reserved pipeline metric id {0:?}")]
    ReservedMetric(String),
    /// Two suites published the same metric.
    #[error("duplicate metric id {0:?}")]
    DuplicateMetric(String),
    /// Two outputs used the same suite identifier.
    #[error("duplicate suite output {0:?}")]
    DuplicateSuite(String),
    /// A skip claimed ownership by a different suite.
    #[error("suite output {output:?} contains a skip owned by {skip:?}")]
    SkipSuiteMismatch {
        /// Output suite label.
        output: String,
        /// Skip suite label.
        skip: String,
    },
    /// One strict derivation input is missing.
    #[error("derivation {target:?} is half-present: {present} exists but {absent} is missing")]
    HalfPresentDerivation {
        /// Derived metric identifier.
        target: &'static str,
        /// Input that exists.
        present: &'static str,
        /// Input that is missing.
        absent: &'static str,
    },
}

#[derive(Clone, Copy, Debug)]
struct Derivation {
    target: &'static str,
    minuend: &'static str,
    subtrahend: &'static str,
    unit: &'static str,
    strict: bool,
}

const DERIVATIONS: &[Derivation] = &[
    Derivation {
        target: "startup.version.over_python_ms",
        minuend: "startup.version.median_ms",
        subtrahend: "startup.python.median_ms",
        unit: "ms",
        strict: true,
    },
    Derivation {
        target: "scale.list_json.per_entry_us",
        minuend: "scale.list_json.n1000.median_ms",
        subtrahend: "scale.list_json.n0.median_ms",
        unit: "us",
        strict: false,
    },
    Derivation {
        target: "run_overhead.python.overhead_ms",
        minuend: "run_overhead.python.skit.median_ms",
        subtrahend: "run_overhead.python.uv_script.median_ms",
        unit: "ms",
        strict: true,
    },
    Derivation {
        target: "run_overhead.shell.overhead_ms",
        minuend: "run_overhead.shell.skit.median_ms",
        subtrahend: "run_overhead.shell.bash.median_ms",
        unit: "ms",
        strict: true,
    },
    Derivation {
        target: "run_overhead.js.overhead_ms",
        minuend: "run_overhead.js.skit.median_ms",
        subtrahend: "run_overhead.js.node.median_ms",
        unit: "ms",
        strict: true,
    },
];

/// Merge suite outputs and compute the cross-suite metrics from latest Python main.
pub fn merge(
    meta: Meta,
    outputs: Vec<SuiteOutput>,
    total_duration_seconds: f64,
) -> Result<Results, PipelineError> {
    meta.validate()
        .map_err(|error| PipelineError::InvalidMeta(error.to_string()))?;
    if !total_duration_seconds.is_finite() || total_duration_seconds < 0.0 {
        return Err(PipelineError::InvalidTotalDuration);
    }
    let mut metrics = std::collections::BTreeMap::new();
    let mut skipped = Vec::new();
    let mut raw = std::collections::BTreeMap::new();

    for output in outputs {
        let suite_id = output.suite.as_str();
        if let Some(skip) = output
            .skipped
            .iter()
            .find(|skip| skip.suite != output.suite)
        {
            return Err(PipelineError::SkipSuiteMismatch {
                output: suite_id.to_owned(),
                skip: skip.suite.as_str().to_owned(),
            });
        }
        output
            .validate()
            .map_err(|error| PipelineError::InvalidSuiteOutput {
                suite: suite_id.to_owned(),
                reason: error.to_string(),
            })?;
        if raw
            .insert(
                suite_id.to_owned(),
                serde_json::Value::Object(output.raw.into_iter().collect()),
            )
            .is_some()
        {
            return Err(PipelineError::DuplicateSuite(suite_id.to_owned()));
        }
        for (metric_id, metric) in output.metrics {
            if metric_id.starts_with("pipeline.") {
                return Err(PipelineError::ReservedMetric(metric_id));
            }
            if metrics.insert(metric_id.clone(), metric).is_some() {
                return Err(PipelineError::DuplicateMetric(metric_id));
            }
        }
        skipped.extend(output.skipped);
        metrics.insert(
            format!("pipeline.suite.{suite_id}.duration_s"),
            Metric::single(round_to(output.duration_seconds, 3), "s"),
        );
    }

    metrics.insert(
        "pipeline.duration_s".to_owned(),
        Metric::single(round_to(total_duration_seconds, 3), "s"),
    );
    metrics.insert(
        "pipeline.skipped_count".to_owned(),
        Metric::single(skipped.len() as f64, "count"),
    );
    for (metric_id, metric) in derive(&metrics)? {
        if metrics.insert(metric_id.clone(), metric).is_some() {
            return Err(PipelineError::DuplicateMetric(metric_id));
        }
    }

    let results = Results {
        schema_version: 1,
        meta,
        metrics,
        skipped,
        raw,
    };
    results
        .validate()
        .map_err(|error| PipelineError::InvalidMergedResults(error.to_string()))?;
    Ok(results)
}

fn derive(
    metrics: &std::collections::BTreeMap<String, Metric>,
) -> Result<Vec<(String, Metric)>, PipelineError> {
    let mut derived = Vec::new();
    for item in DERIVATIONS {
        let minuend = metrics.get(item.minuend);
        let subtrahend = metrics.get(item.subtrahend);
        match (minuend, subtrahend) {
            (Some(minuend), Some(subtrahend)) => derived.push((
                item.target.to_owned(),
                Metric::single(round_to(minuend.value - subtrahend.value, 4), item.unit),
            )),
            (Some(_), None) if item.strict => {
                return Err(PipelineError::HalfPresentDerivation {
                    target: item.target,
                    present: item.minuend,
                    absent: item.subtrahend,
                });
            }
            (None, Some(_)) if item.strict => {
                return Err(PipelineError::HalfPresentDerivation {
                    target: item.target,
                    present: item.subtrahend,
                    absent: item.minuend,
                });
            }
            (Some(_), None) | (None, Some(_)) | (None, None) => {}
        }
    }
    Ok(derived)
}

fn round_to(value: f64, decimal_places: i32) -> f64 {
    let factor = 10_f64.powi(decimal_places);
    let scaled = value * factor;
    if scaled.is_finite() {
        scaled.round() / factor
    } else {
        value
    }
}
