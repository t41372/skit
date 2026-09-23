//! Two-tier benchmark budget loading, evaluation, and ratchet proposals.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use thiserror::Error;

use crate::Results;

/// Ratchet bounds below this fraction are reported as stale.
pub const STALE_FRACTION: f64 = 0.85;
const DEFAULT_HEADROOM: f64 = 0.10;

const HEADER: &str = r#"# The performance contract (docs/design/benchmarks.md — "Budgets").
#
# tier = "enforced": `skit-bench check` fails on violation. Use this tier only
#   for deterministic or ratchet-safe metrics. Every row has provenance context.
# tier = "target": the aspirational contract. Each run reports it, but it does
#   not fail CI until a future change deliberately moves the row to enforced.
# ratchet = true: the bound comes from a measured value plus headroom.
#   `skit-bench check --propose` refreshes it. Use CI artifacts only. The command
#   refuses a local run or a dirty tree because the census depends on the platform
#   and Python version. It also refuses to widen a bound unless you pass
#   --allow-regression. Propose leaves non-ratchet contract values unchanged.
# context.pr / context.commit: the source of a refreshed bound. A pull-request
#   artifact uses the PR number because GitHub's ephemeral merge commit does not
#   remain available. Other artifacts use the measured commit.
#
# Regenerate this file with `skit-bench check results.json --propose`. Keep prose
# in `note` fields, not in comments below this header.
"#;

/// Budget severity.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetTier {
    /// A failed row fails the check.
    Enforced,
    /// A failed row is an advisory target.
    Target,
}

/// One performance contract row.
#[derive(Clone, Debug, PartialEq)]
pub struct Budget {
    /// Stable metric identifier.
    pub metric: String,
    /// Inclusive ceiling.
    pub max_value: f64,
    /// Failure policy.
    pub tier: BudgetTier,
    /// Whether propose derives this bound from a measurement.
    pub ratchet: bool,
    /// Fractional room above the measured value.
    pub headroom: f64,
    /// Empty means every profile.
    pub profiles: Vec<String>,
    /// Optional platform predicate.
    pub platform: Option<String>,
    /// Apply only when the artifact records a CI runner.
    pub ci_only: bool,
    /// Provenance for the bound.
    pub context: BTreeMap<String, String>,
    /// Human rationale.
    pub note: String,
}

/// One budget verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetOutcome {
    /// Metric was at or below its ceiling.
    Passed,
    /// Metric exceeded its ceiling.
    Violated,
    /// Metric did not exist.
    MetricMissing,
    /// A predicate excluded this artifact.
    NotApplicable,
    /// Metadata required by a predicate was empty.
    PredicateUnevaluable,
    /// A CI Python version did not match ratchet provenance.
    PythonMismatch,
}

impl BudgetOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Violated => "violated",
            Self::MetricMissing => "metric-missing",
            Self::NotApplicable => "not-applicable",
            Self::PredicateUnevaluable => "predicate-unevaluable",
            Self::PythonMismatch => "python-mismatch",
        }
    }
}

/// Evaluation of one row.
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetRowResult {
    /// Contract row.
    pub budget: Budget,
    /// Verdict.
    pub outcome: BudgetOutcome,
    /// Measured value when one was evaluated.
    pub value: Option<f64>,
    /// Human detail.
    pub detail: String,
    /// Whether a ratchet should be tightened.
    pub stale: bool,
}

impl BudgetRowResult {
    /// Whether this verdict fails the command.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.budget.tier == BudgetTier::Enforced
            && matches!(
                self.outcome,
                BudgetOutcome::Violated
                    | BudgetOutcome::MetricMissing
                    | BudgetOutcome::PredicateUnevaluable
                    | BudgetOutcome::PythonMismatch
            )
    }
}

/// Complete budget report.
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetReport {
    /// Results in contract-file order.
    pub rows: Vec<BudgetRowResult>,
}

impl BudgetReport {
    /// Failed enforced rows.
    #[must_use]
    pub fn failures(&self) -> Vec<&BudgetRowResult> {
        self.rows.iter().filter(|row| row.failed()).collect()
    }

    /// Enforced rows that reached any verdict except not-applicable.
    #[must_use]
    pub fn enforced_evaluated(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| {
                row.budget.tier == BudgetTier::Enforced
                    && row.outcome != BudgetOutcome::NotApplicable
            })
            .count()
    }
}

/// A budget file or proposal was invalid.
#[derive(Debug, Error)]
pub enum BudgetError {
    /// TOML parse failure.
    #[error("budgets.toml is not valid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    /// TOML serialization failure.
    #[error("could not serialize budgets.toml: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// Contract invariant failed.
    #[error("{0}")]
    Invalid(String),
}

/// Load and strictly validate every `[[budget]]` row.
pub fn load_budgets(text: &str) -> Result<Vec<Budget>, BudgetError> {
    let document: toml::Table = toml::from_str(text)?;
    let unknown = document
        .keys()
        .filter(|key| key.as_str() != "budget")
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(BudgetError::Invalid(format!(
            "unknown top-level keys: {unknown:?}"
        )));
    }
    let rows = document
        .get("budget")
        .and_then(toml::Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| BudgetError::Invalid("no [[budget]] rows".to_owned()))?;
    rows.iter()
        .enumerate()
        .map(|(index, value)| load_row(index, value))
        .collect()
}

fn load_row(index: usize, value: &toml::Value) -> Result<Budget, BudgetError> {
    let row = value.as_table().ok_or_else(|| {
        BudgetError::Invalid(format!("[[budget]] #{}: expected a table", index + 1))
    })?;
    const ALLOWED: &[&str] = &[
        "metric", "max", "tier", "ratchet", "headroom", "profiles", "platform", "ci_only",
        "context", "note",
    ];
    let allowed = ALLOWED.iter().copied().collect::<BTreeSet<_>>();
    let unknown = row
        .keys()
        .filter(|key| !allowed.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(BudgetError::Invalid(format!(
            "[[budget]] #{}: unknown keys {unknown:?}",
            index + 1
        )));
    }
    let metric = non_empty_string(row, "metric", index)?;
    let where_ = format!("[[budget]] {metric}");
    let max_value = number(row.get("max"))
        .ok_or_else(|| BudgetError::Invalid(format!("{where_}: max must be a finite number")))?;
    let tier = match row.get("tier").and_then(toml::Value::as_str) {
        Some("enforced") => BudgetTier::Enforced,
        Some("target") => BudgetTier::Target,
        _ => {
            return Err(BudgetError::Invalid(format!(
                "{where_}: tier must be 'enforced' or 'target'"
            )));
        }
    };
    let ratchet = bool_value(row, "ratchet", false, &where_)?;
    if ratchet && tier != BudgetTier::Enforced {
        return Err(BudgetError::Invalid(format!(
            "{where_}: ratchet rows must be enforced"
        )));
    }
    let headroom = match row.get("headroom") {
        None => DEFAULT_HEADROOM,
        value => number(value)
            .ok_or_else(|| BudgetError::Invalid(format!("{where_}: headroom must be a number")))?,
    };
    if !(0.0..1.0).contains(&headroom) || headroom == 0.0 {
        return Err(BudgetError::Invalid(format!(
            "{where_}: headroom must be between 0 and 1"
        )));
    }
    let profiles = match row.get("profiles") {
        None => Vec::new(),
        Some(toml::Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        BudgetError::Invalid(format!(
                            "{where_}: profiles must be an array of non-empty strings"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(BudgetError::Invalid(format!(
                "{where_}: profiles must be an array of non-empty strings"
            )));
        }
    };
    let platform = optional_string(row, "platform", &where_)?;
    let ci_only = bool_value(row, "ci_only", false, &where_)?;
    let context = match row.get("context") {
        None => BTreeMap::new(),
        Some(toml::Value::Table(values)) => values
            .iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|value| (key.clone(), value.to_owned()))
                    .ok_or_else(|| {
                        BudgetError::Invalid(format!(
                            "{where_}: context must be a table of strings"
                        ))
                    })
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(BudgetError::Invalid(format!(
                "{where_}: context must be a table of strings"
            )));
        }
    };
    if tier == BudgetTier::Enforced && context.is_empty() {
        return Err(BudgetError::Invalid(format!(
            "{where_}: enforced rows require context (provenance)"
        )));
    }
    let note = match row.get("note") {
        None => String::new(),
        Some(toml::Value::String(value)) => value.clone(),
        Some(_) => {
            return Err(BudgetError::Invalid(format!(
                "{where_}: note must be a string"
            )));
        }
    };
    Ok(Budget {
        metric,
        max_value,
        tier,
        ratchet,
        headroom,
        profiles,
        platform,
        ci_only,
        context,
        note,
    })
}

fn non_empty_string(row: &toml::Table, key: &str, index: usize) -> Result<String, BudgetError> {
    row.get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            BudgetError::Invalid(format!(
                "[[budget]] #{}: {key} must be a non-empty string",
                index + 1
            ))
        })
}

fn optional_string(
    row: &toml::Table,
    key: &str,
    where_: &str,
) -> Result<Option<String>, BudgetError> {
    match row.get(key) {
        None => Ok(None),
        Some(toml::Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(_) => Err(BudgetError::Invalid(format!(
            "{where_}: {key} must be a non-empty string"
        ))),
    }
}

fn bool_value(
    row: &toml::Table,
    key: &str,
    default: bool,
    where_: &str,
) -> Result<bool, BudgetError> {
    match row.get(key) {
        None => Ok(default),
        Some(toml::Value::Boolean(value)) => Ok(*value),
        Some(_) => Err(BudgetError::Invalid(format!(
            "{where_}: {key} must be a boolean"
        ))),
    }
}

fn number(value: Option<&toml::Value>) -> Option<f64> {
    let value = match value? {
        toml::Value::Integer(value) => *value as f64,
        toml::Value::Float(value) => *value,
        _ => return None,
    };
    value.is_finite().then_some(value)
}

/// Evaluate budgets in file order.
#[must_use]
pub fn evaluate(budgets: &[Budget], results: &Results) -> BudgetReport {
    BudgetReport {
        rows: budgets
            .iter()
            .map(|budget| evaluate_row(budget, results))
            .collect(),
    }
}

fn evaluate_row(budget: &Budget, results: &Results) -> BudgetRowResult {
    let meta = &results.meta;
    if !budget.profiles.is_empty()
        && !budget
            .profiles
            .iter()
            .any(|profile| profile == meta.profile.as_str())
    {
        return row(
            budget,
            BudgetOutcome::NotApplicable,
            None,
            format!("profile is {}", meta.profile.as_str()),
            false,
        );
    }
    if let Some(platform) = &budget.platform {
        if meta.host.platform_key.is_empty() {
            return row(
                budget,
                BudgetOutcome::PredicateUnevaluable,
                None,
                "meta.host.platform_key is empty".to_owned(),
                false,
            );
        }
        if platform != &meta.host.platform_key {
            return row(
                budget,
                BudgetOutcome::NotApplicable,
                None,
                format!("platform is {}", meta.host.platform_key),
                false,
            );
        }
    }
    let on_ci = meta.host.ci_runner.is_some();
    if meta.host.ci_runner.as_deref() == Some("") {
        return row(
            budget,
            BudgetOutcome::PredicateUnevaluable,
            None,
            "meta.host.ci_runner is empty".to_owned(),
            false,
        );
    }
    if budget.ci_only && !on_ci {
        return row(
            budget,
            BudgetOutcome::NotApplicable,
            None,
            "not a CI run".to_owned(),
            false,
        );
    }
    if budget.tier == BudgetTier::Enforced
        && let Some(pinned) = budget
            .context
            .get("python")
            .filter(|value| !value.is_empty())
    {
        let actual = python_major_minor(&meta.python);
        if pinned != &actual {
            let outcome = if on_ci {
                BudgetOutcome::PythonMismatch
            } else {
                BudgetOutcome::NotApplicable
            };
            let detail = if on_ci {
                format!("bound set on python {pinned}, CI runs {actual}")
            } else {
                format!("bound pinned to python {pinned}; this host runs {actual}")
            };
            return row(budget, outcome, None, detail, false);
        }
    }
    let Some(metric) = results.metrics.get(&budget.metric) else {
        return row(
            budget,
            BudgetOutcome::MetricMissing,
            None,
            "metric absent from results".to_owned(),
            false,
        );
    };
    if metric.value > budget.max_value {
        return row(
            budget,
            BudgetOutcome::Violated,
            Some(metric.value),
            format!(
                "{} {} > {}",
                format_number(metric.value),
                metric.unit,
                format_number(budget.max_value)
            ),
            false,
        );
    }
    row(
        budget,
        BudgetOutcome::Passed,
        Some(metric.value),
        format!(
            "{} {} ≤ {}",
            format_number(metric.value),
            metric.unit,
            format_number(budget.max_value)
        ),
        budget.ratchet && metric.value < STALE_FRACTION * budget.max_value,
    )
}

fn row(
    budget: &Budget,
    outcome: BudgetOutcome,
    value: Option<f64>,
    detail: String,
    stale: bool,
) -> BudgetRowResult {
    BudgetRowResult {
        budget: budget.clone(),
        outcome,
        value,
        detail,
        stale,
    }
}

/// Render the human check report.
#[must_use]
pub fn render_report(report: &BudgetReport) -> String {
    let mut lines = Vec::new();
    for row in &report.rows {
        let mark = if row.budget.tier == BudgetTier::Target {
            if row.outcome == BudgetOutcome::Passed {
                "ok"
            } else {
                "△"
            }
        } else if row.failed() {
            "FAIL"
        } else if row.outcome == BudgetOutcome::NotApplicable {
            "n/a"
        } else {
            "ok"
        };
        lines.push(format!(
            "[{}] {mark:>4}  {}: {}{}",
            if row.budget.tier == BudgetTier::Enforced {
                "enforced"
            } else {
                "target"
            },
            row.budget.metric,
            row.outcome.as_str(),
            if row.detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", row.detail)
            }
        ));
        if row.stale {
            lines.push(format!(
                "[enforced] note  {}: measured {} sits below {:.0}% of the bound {} — ceiling is stale, tighten it (check --propose)",
                row.budget.metric,
                format_number(row.value.unwrap_or_default()),
                STALE_FRACTION * 100.0,
                format_number(row.budget.max_value)
            ));
        }
    }
    let enforced = report
        .rows
        .iter()
        .filter(|row| row.budget.tier == BudgetTier::Enforced)
        .collect::<Vec<_>>();
    let passed = enforced
        .iter()
        .filter(|row| row.outcome == BudgetOutcome::Passed)
        .count();
    let targets = report.rows.len() - enforced.len();
    lines.push(format!(
        "enforced: {} rows, {} evaluated, {passed} passed, {} failed · target: {targets} rows",
        enforced.len(),
        report.enforced_evaluated(),
        report.failures().len()
    ));
    lines.push(String::new());
    lines.join("\n")
}

/// Refresh ratchet bounds from a clean CI artifact.
pub fn propose(
    budgets: &[Budget],
    results: &Results,
    allow_regression: bool,
) -> Result<String, BudgetError> {
    if results.meta.host.ci_runner.is_none() {
        return Err(BudgetError::Invalid(
            "cannot propose from a local run: ratchet bounds come from CI artifacts (the census is platform- and python-dependent). Download the run's benchmark-results-* artifact and propose from that results.json.".to_owned(),
        ));
    }
    if results.meta.git.dirty {
        return Err(BudgetError::Invalid(
            "cannot propose from a dirty tree: the bound would record a commit that does not describe what was measured".to_owned(),
        ));
    }
    let provenance = provenance(results);
    let mut refreshed = Vec::with_capacity(budgets.len());
    let mut widened = Vec::new();
    for budget in budgets {
        if !budget.ratchet {
            refreshed.push(budget.clone());
            continue;
        }
        let metric = results.metrics.get(&budget.metric).ok_or_else(|| {
            BudgetError::Invalid(format!(
                "cannot propose {}: metric absent from results",
                budget.metric
            ))
        })?;
        let bound = (metric.value * (1.0 + budget.headroom)).ceil();
        if bound > budget.max_value {
            widened.push(format!(
                "{}: {} -> {} (measured {})",
                budget.metric,
                format_number(budget.max_value),
                format_number(bound),
                format_number(metric.value)
            ));
        }
        let mut proposed = budget.clone();
        proposed.max_value = bound;
        proposed.context.clone_from(&provenance);
        refreshed.push(proposed);
    }
    if !widened.is_empty() && !allow_regression {
        return Err(BudgetError::Invalid(format!(
            "refusing to loosen enforced bounds — this artifact regressed against the committed contract:\n  {}\nFix the regression, or pass --allow-regression if the increase is intended (say why in the row's note).",
            widened.join("\n  ")
        )));
    }
    render_budgets(&refreshed)
}

fn provenance(results: &Results) -> BTreeMap<String, String> {
    let mut context = BTreeMap::from([
        (
            "python".to_owned(),
            python_major_minor(&results.meta.python),
        ),
        (
            "date".to_owned(),
            results
                .meta
                .generated_at
                .split('T')
                .next()
                .unwrap_or(&results.meta.generated_at)
                .to_owned(),
        ),
    ]);
    if let Some(pr) = &results.meta.git.pr {
        context.insert("pr".to_owned(), pr.clone());
    } else {
        context.insert("commit".to_owned(), results.meta.git.commit.clone());
    }
    context
}

#[derive(Serialize)]
struct BudgetDocument<'a> {
    budget: Vec<SerializableBudget<'a>>,
}

#[derive(Serialize)]
struct SerializableBudget<'a> {
    metric: &'a str,
    #[serde(rename = "max")]
    max_value: toml::Value,
    tier: BudgetTier,
    #[serde(skip_serializing_if = "is_false")]
    ratchet: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    headroom: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    profiles: &'a Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: &'a Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    ci_only: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    context: &'a BTreeMap<String, String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    note: &'a String,
}

const fn is_false(value: &bool) -> bool {
    !*value
}

/// Serialize the canonical budget document.
pub fn render_budgets(budgets: &[Budget]) -> Result<String, BudgetError> {
    let budget = budgets
        .iter()
        .map(|row| SerializableBudget {
            metric: &row.metric,
            max_value: if row.max_value.fract() == 0.0
                && row.max_value >= i64::MIN as f64
                && row.max_value <= i64::MAX as f64
            {
                toml::Value::Integer(row.max_value as i64)
            } else {
                toml::Value::Float(row.max_value)
            },
            tier: row.tier,
            ratchet: row.ratchet,
            headroom: (row.ratchet || row.headroom != DEFAULT_HEADROOM).then_some(row.headroom),
            profiles: &row.profiles,
            platform: &row.platform,
            ci_only: row.ci_only,
            context: &row.context,
            note: &row.note,
        })
        .collect();
    Ok(format!(
        "{HEADER}\n{}",
        toml::to_string_pretty(&BudgetDocument { budget })?
    ))
}

/// Return major.minor provenance granularity.
#[must_use]
pub fn python_major_minor(version: &str) -> String {
    version.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// Render an integer without exponent notation, or a compact finite float.
#[must_use]
pub fn format_number(value: f64) -> String {
    if value == 0.0 {
        "0".to_owned()
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}
