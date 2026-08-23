//! Fresh-process headless terminal interaction measurements.

use std::{collections::BTreeMap, path::Path};

use crate::{
    Metric, Skip, SuiteKind, SuiteOutput, SuitePlan,
    parsers::vmhwm_kib,
    process::{ProcessSpec, run as run_process},
    runner::{PROBE_TIMEOUT, RunContext, path_arg},
    stats::{median, nearest_rank_p95, sample_stddev},
    tui_probe::ProbeResult,
};

use super::SuiteError;

pub(super) fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    run_with_status(context, plan, Path::new("/proc/self/status").is_file())
}

fn run_with_status(
    context: &RunContext,
    plan: &SuitePlan,
    has_status: bool,
) -> Result<SuiteOutput, SuiteError> {
    let mut output = SuiteOutput {
        suite: SuiteKind::Tui,
        duration_seconds: 0.0,
        metrics: BTreeMap::new(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    if !has_status {
        output.skipped.push(Skip {
            suite: SuiteKind::Tui,
            case: "peak_rss".to_owned(),
            reason: "no /proc/self/status on this host".to_owned(),
        });
    }
    let mut import_samples = Vec::new();
    for n in &plan.library_sizes {
        let dataset = context.dataset(*n)?;
        let probe_char = dataset
            .probe_char
            .chars()
            .next()
            .ok_or_else(|| SuiteError::Contract("empty TUI probe character".to_owned()))?;
        let mut first_idle = Vec::new();
        let mut select = Vec::new();
        let mut search = Vec::new();
        let mut peaks = Vec::new();
        for sample in 0..plan.samples {
            let result = run_process(&ProcessSpec {
                argv: vec![
                    path_arg(&context.harness),
                    "probe".to_owned(),
                    "tui".to_owned(),
                    "--entries".to_owned(),
                    n.to_string(),
                    "--probe-char".to_owned(),
                    probe_char.to_string(),
                ],
                cwd: context.workdir.clone(),
                env: context.environment(*n)?,
                timeout: PROBE_TIMEOUT,
                check: true,
            });
            let process = match result {
                Ok(process) => process,
                Err(error) if plan.compare_mode => {
                    output.skipped.push(Skip {
                        suite: SuiteKind::Tui,
                        case: format!("n{n}.sample{sample}"),
                        reason: error.to_string(),
                    });
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            let probe: ProbeResult = serde_json::from_slice(&process.stdout).map_err(|error| {
                SuiteError::Contract(format!("invalid TUI probe JSON for n{n}: {error}"))
            })?;
            probe
                .validate()
                .map_err(|error| SuiteError::Contract(error.to_string()))?;
            first_idle.push(probe.first_idle_ms);
            search.push(probe.search_ms);
            if let Some(value) = probe.select_ms {
                select.push(value);
            }
            if *n == 0 {
                import_samples.push(0.0);
            }
            if has_status {
                let text = probe.status_text.ok_or_else(|| {
                    SuiteError::Contract(
                        "TUI probe omitted /proc/self/status on a Linux procfs host".to_owned(),
                    )
                })?;
                peaks.push(vmhwm_kib(&text)? as f64);
            }
        }
        if first_idle.is_empty() {
            continue;
        }
        insert_stat(
            &mut output,
            format!("tui.first_idle.n{n}.median_ms"),
            &first_idle,
            "ms",
        )?;
        insert_stat(
            &mut output,
            format!("tui.search.n{n}.median_ms"),
            &search,
            "ms",
        )?;
        if !select.is_empty() {
            insert_stat(
                &mut output,
                format!("tui.select.n{n}.median_ms"),
                &select,
                "ms",
            )?;
        }
        if !peaks.is_empty() {
            insert_stat(&mut output, format!("tui.rss.n{n}.peak_kib"), &peaks, "KiB")?;
        }
        output.raw.insert(
            format!("n{n}"),
            raw_case(
                &first_idle,
                &select,
                &search,
                &peaks,
                (*n == 0).then_some(import_samples.as_slice()),
            ),
        );
    }
    if !import_samples.is_empty() {
        insert_stat(
            &mut output,
            "tui.import.median_ms".to_owned(),
            &import_samples,
            "ms",
        )?;
        output.raw.insert(
            "native_import_note".to_owned(),
            serde_json::json!("not applicable: Rust binary has no Python import path"),
        );
    }
    Ok(output)
}

fn raw_case(
    first_idle: &[f64],
    select: &[f64],
    search: &[f64],
    peaks: &[f64],
    import_samples: Option<&[f64]>,
) -> serde_json::Value {
    let mut raw = serde_json::Map::from_iter([
        ("first_idle_ms".to_owned(), serde_json::json!(first_idle)),
        ("select_ms".to_owned(), serde_json::json!(select)),
        ("search_ms".to_owned(), serde_json::json!(search)),
        ("rss_kib".to_owned(), serde_json::json!(peaks)),
    ]);
    if let Some(samples) = import_samples {
        raw.insert("import_ms".to_owned(), serde_json::json!(samples));
    }
    serde_json::Value::Object(raw)
}

fn insert_stat(
    output: &mut SuiteOutput,
    metric_id: String,
    samples: &[f64],
    unit: &str,
) -> Result<(), SuiteError> {
    output.metrics.insert(
        metric_id,
        Metric {
            value: median(samples)?,
            unit: unit.to_owned(),
            n: samples.len(),
            p95: Some(nearest_rank_p95(samples)?),
            stddev: Some(sample_stddev(samples)?),
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn test_tui_keeps_import_and_rss_samples() {
        use crate::{
            SuiteKind, SuiteOutput,
            suites::tests::{Fixture, plan},
        };

        let fixture = Fixture::new();
        let mut plan = plan(SuiteKind::Tui, &[0]);
        plan.samples = 3;
        let output = super::run_with_status(&fixture.context, &plan, true).unwrap();
        let output = SuiteOutput::from_json(&output.to_json().unwrap()).unwrap();

        assert_eq!(
            output.raw["n0"],
            serde_json::json!({
                "first_idle_ms": [1.5, 1.5, 1.5],
                "select_ms": [],
                "search_ms": [0.75, 0.75, 0.75],
                "rss_kib": [1234.0, 1234.0, 1234.0],
                "import_ms": [0.0, 0.0, 0.0],
            })
        );
        assert!(!output.metrics.contains_key("tui.select.n0.median_ms"));
        let rss = &output.metrics["tui.rss.n0.peak_kib"];
        assert_eq!(rss.n, 3);
        assert_eq!(rss.p95, Some(1234.0));
        assert_eq!(rss.stddev, Some(0.0));
        assert_eq!(output.metrics["tui.import.median_ms"].n, 3);
    }

    #[cfg(unix)]
    #[test]
    fn test_tui_records_the_selection_span_when_the_probe_measured_one() {
        use crate::{
            SuiteKind, SuiteOutput,
            suites::tests::{Fixture, plan},
        };

        let fixture = Fixture::new();
        let mut plan = plan(SuiteKind::Tui, &[100]);
        plan.samples = 3;
        let output = super::run_with_status(&fixture.context, &plan, false).unwrap();
        let output = SuiteOutput::from_json(&output.to_json().unwrap()).unwrap();

        assert_eq!(
            output.raw["n100"]["select_ms"],
            serde_json::json!([0.25, 0.25, 0.25])
        );
        let selection = &output.metrics["tui.select.n100.median_ms"];
        assert_eq!(selection.value, 0.25);
        assert_eq!(selection.n, 3);
        assert_eq!(selection.p95, Some(0.25));
    }

    #[test]
    fn raw_case_omits_import_samples_outside_the_zero_entry_probe() {
        let without_import = super::raw_case(&[1.0], &[], &[2.0], &[3.0], None);
        assert!(without_import.get("import_ms").is_none());

        let with_import = super::raw_case(&[1.0], &[], &[2.0], &[3.0], Some(&[0.0]));
        assert_eq!(with_import["import_ms"], serde_json::json!([0.0]));
    }

    #[cfg(unix)]
    #[test]
    fn child_failures_corrupt_payloads_and_missing_procfs_are_explicit() {
        use crate::{
            SuiteKind,
            suites::tests::{Fixture, executable, plan},
        };

        let mut fixture = Fixture::new();
        let mut compare = plan(SuiteKind::Tui, &[0]);
        compare.samples = 1;
        compare.compare_mode = true;
        fixture.context.harness = executable(
            &fixture.context.workdir.join("failed-tui"),
            "#!/bin/sh\nexit 7\n",
        );
        let skipped = super::run_with_status(&fixture.context, &compare, true).unwrap();
        assert_eq!(skipped.skipped[0].case, "n0.sample0");
        assert!(skipped.metrics.is_empty());

        let mut strict = compare.clone();
        strict.compare_mode = false;
        assert!(super::run_with_status(&fixture.context, &strict, true).is_err());

        fixture.context.harness = executable(
            &fixture.context.workdir.join("invalid-tui"),
            "#!/bin/sh\nprintf 'not-json\\n'\n",
        );
        assert!(super::run_with_status(&fixture.context, &strict, true).is_err());

        fixture.context.harness = executable(
            &fixture.context.workdir.join("nonfinite-tui"),
            "#!/bin/sh\nprintf '{\"first_idle_ms\":-1,\"select_ms\":null,\"search_ms\":1,\"status_text\":null}\\n'\n",
        );
        assert!(super::run_with_status(&fixture.context, &strict, false).is_err());

        fixture.context.harness = executable(
            &fixture.context.workdir.join("without-status-tui"),
            "#!/bin/sh\nprintf '{\"first_idle_ms\":1,\"select_ms\":null,\"search_ms\":2,\"status_text\":null}\\n'\n",
        );
        let no_procfs = super::run_with_status(&fixture.context, &strict, false).unwrap();
        assert_eq!(no_procfs.skipped[0].case, "peak_rss");
        assert!(
            no_procfs
                .metrics
                .contains_key("tui.first_idle.n0.median_ms")
        );
        assert!(super::run_with_status(&fixture.context, &strict, true).is_err());

        fixture
            .context
            .datasets
            .get_mut(&0)
            .unwrap()
            .probe_char
            .clear();
        assert!(super::run_with_status(&fixture.context, &strict, false).is_err());
    }
}
