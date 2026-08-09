//! Warn-only A/B comparison reports.

use crate::{Results, budget::format_number};

const RELATIVE_THRESHOLD: f64 = 0.05;

/// One metric delta.
#[derive(Clone, Debug, PartialEq)]
pub struct Delta {
    /// Metric identifier.
    pub metric: String,
    /// Shared unit.
    pub unit: String,
    /// Base value.
    pub base: f64,
    /// Head value.
    pub head: f64,
}

impl Delta {
    /// Absolute signed change.
    #[must_use]
    pub fn difference(&self) -> f64 {
        self.head - self.base
    }

    /// Percentage change, absent when base is zero.
    #[must_use]
    pub fn percent(&self) -> Option<f64> {
        (self.base != 0.0).then(|| self.difference() / self.base * 100.0)
    }

    /// Whether the delta clears the per-unit noise policy.
    #[must_use]
    pub fn is_notable(&self) -> bool {
        if matches!(self.unit.as_str(), "count" | "bytes" | "bool") {
            return self.difference() != 0.0;
        }
        let floor = match self.unit.as_str() {
            "s" => 0.002,
            "ms" => 2.0,
            "us" => 1.0,
            _ => 0.0,
        };
        if self.difference().abs() <= floor {
            return false;
        }
        if self.base == 0.0 {
            return self.head != 0.0;
        }
        self.difference().abs() > RELATIVE_THRESHOLD * self.base.abs()
    }
}

/// Complete comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    /// Metrics available on both sides with matching units.
    pub deltas: Vec<Delta>,
    /// Metrics only in base.
    pub only_base: Vec<String>,
    /// Metrics only in head.
    pub only_head: Vec<String>,
    /// Provenance and unit mismatches.
    pub incomparable: Vec<String>,
}

impl Comparison {
    /// Deltas that clear the noise policy.
    #[must_use]
    pub fn notable(&self) -> Vec<&Delta> {
        self.deltas
            .iter()
            .filter(|delta| delta.is_notable())
            .collect()
    }
}

/// Compare two validated artifacts without gating them.
#[must_use]
pub fn compare(base: &Results, head: &Results) -> Comparison {
    let mut incomparable = Vec::new();
    mismatch(
        &mut incomparable,
        "profile",
        base.meta.profile.as_str(),
        head.meta.profile.as_str(),
    );
    mismatch(
        &mut incomparable,
        "platform",
        &base.meta.host.platform_key,
        &head.meta.host.platform_key,
    );
    mismatch_option(
        &mut incomparable,
        "runner image",
        base.meta.host.ci_image_version.as_deref(),
        head.meta.host.ci_image_version.as_deref(),
    );
    mismatch(
        &mut incomparable,
        "python",
        &crate::budget::python_major_minor(&base.meta.python),
        &crate::budget::python_major_minor(&head.meta.python),
    );
    mismatch(
        &mut incomparable,
        "pyperf",
        &base.meta.pyperf,
        &head.meta.pyperf,
    );

    let mut deltas = Vec::new();
    for (metric_id, base_metric) in &base.metrics {
        if metric_id.starts_with("pipeline.") {
            continue;
        }
        let Some(head_metric) = head.metrics.get(metric_id) else {
            continue;
        };
        if base_metric.unit != head_metric.unit {
            incomparable.push(format!(
                "unit {metric_id}: {} vs {}",
                base_metric.unit, head_metric.unit
            ));
            continue;
        }
        deltas.push(Delta {
            metric: metric_id.clone(),
            unit: head_metric.unit.clone(),
            base: base_metric.value,
            head: head_metric.value,
        });
    }
    let mut only_base = base
        .metrics
        .keys()
        .filter(|id| !id.starts_with("pipeline.") && !head.metrics.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let mut only_head = head
        .metrics
        .keys()
        .filter(|id| !id.starts_with("pipeline.") && !base.metrics.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    only_base.sort();
    only_head.sort();
    Comparison {
        deltas,
        only_base,
        only_head,
        incomparable,
    }
}

fn mismatch(output: &mut Vec<String>, label: &str, base: &str, head: &str) {
    if base != head {
        output.push(format!("{label}: {base} vs {head}"));
    }
}

fn mismatch_option(output: &mut Vec<String>, label: &str, base: Option<&str>, head: Option<&str>) {
    if base != head {
        output.push(format!(
            "{label}: {} vs {}",
            base.unwrap_or("None"),
            head.unwrap_or("None")
        ));
    }
}

/// Render the evidence report.
#[must_use]
pub fn render_markdown(base: &Results, head: &Results, comparison: &Comparison) -> String {
    let mut lines = vec![
        "## Benchmark comparison".to_owned(),
        String::new(),
        format!(
            "Base: `{}` ({}) · Head: `{}` ({}) · profile {} · {}",
            prefix(&base.meta.git.commit, 12),
            base.meta.skit_version,
            prefix(&head.meta.git.commit, 12),
            head.meta.skit_version,
            head.meta.profile.as_str(),
            head.meta.host.platform_key
        ),
        String::new(),
        "Warn-only: notable = |Δ| > max(5%, per-unit floor: 2 ms macro / 1 µs micro); counts, byte sizes and booleans are exact — any change is notable. Hosted-runner numbers are advisory.".to_owned(),
        String::new(),
    ];
    if !comparison.incomparable.is_empty() {
        lines.extend([
            "> [!WARNING]".to_owned(),
            format!(
                "> **The sides are not directly comparable** — {}. Deltas below mix apples and oranges.",
                comparison.incomparable.join("; ")
            ),
            String::new(),
        ]);
    }
    let notable = comparison.notable();
    lines.push(if notable.is_empty() {
        "### Notable (none)".to_owned()
    } else {
        format!("### Notable ({})", notable.len())
    });
    if !notable.is_empty() {
        lines.extend(table(notable.into_iter()));
    }
    let noise = comparison
        .deltas
        .iter()
        .filter(|delta| !delta.is_notable())
        .collect::<Vec<_>>();
    if !noise.is_empty() {
        lines.extend([
            String::new(),
            format!("<details><summary>Within noise ({})</summary>", noise.len()),
            String::new(),
        ]);
        lines.extend(table(noise.into_iter()));
        lines.extend([String::new(), "</details>".to_owned()]);
    }
    append_only(&mut lines, "Only in base", &comparison.only_base);
    append_only(&mut lines, "Only in head", &comparison.only_head);
    append_skips(&mut lines, "base", base);
    append_skips(&mut lines, "head", head);
    lines.push(String::new());
    lines.join("\n")
}

fn table<'a>(deltas: impl Iterator<Item = &'a Delta>) -> Vec<String> {
    let mut lines = vec![
        "| Metric | Base | Head | Δ | Δ% |".to_owned(),
        "| --- | ---: | ---: | ---: | ---: |".to_owned(),
    ];
    for delta in deltas {
        let percent = delta
            .percent()
            .map_or_else(|| "—".to_owned(), |value| format!("{value:+.1}%"));
        lines.push(format!(
            "| `{}` | {} {} | {} {} | {} | {percent} |",
            delta.metric,
            format_number(delta.base),
            delta.unit,
            format_number(delta.head),
            delta.unit,
            format_signed(delta.difference())
        ));
    }
    lines
}

fn format_signed(value: f64) -> String {
    let sign = if value < 0.0 { '-' } else { '+' };
    format!("{sign}{}", format_number(value.abs()))
}

fn append_only(lines: &mut Vec<String>, title: &str, metrics: &[String]) {
    if metrics.is_empty() {
        return;
    }
    lines.extend([String::new(), format!("### {title}"), String::new()]);
    lines.extend(metrics.iter().map(|metric| format!("- `{metric}`")));
}

fn append_skips(lines: &mut Vec<String>, label: &str, results: &Results) {
    if results.skipped.is_empty() {
        return;
    }
    lines.extend([
        String::new(),
        format!("### Skipped in {label} ({})", results.skipped.len()),
        String::new(),
    ]);
    lines.extend(
        results
            .skipped
            .iter()
            .map(|skip| format!("- `{}/{}`: {}", skip.suite.as_str(), skip.case, skip.reason)),
    );
}

fn prefix(value: &str, length: usize) -> &str {
    value.get(..length).unwrap_or(value)
}
