//! Peak resident memory of fresh CLI processes.

use std::{collections::BTreeMap, fs, path::Path};

use crate::{
    Metric, SuiteKind, SuiteOutput, SuitePlan,
    process::{ProcessSpec, run as run_process},
    runner::{PROBE_TIMEOUT, RunContext, path_arg},
    stats::{median, nearest_rank_p95, sample_stddev},
};

use super::SuiteError;

#[cfg(target_os = "macos")]
use crate::parsers::bsd_time_max_kib;
#[cfg(not(target_os = "macos"))]
use crate::parsers::gnu_time_max_kib;

pub(super) fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    #[cfg(windows)]
    {
        Ok(SuiteOutput::skip_all(
            SuiteKind::Rss,
            "peak RSS probe is POSIX-only",
        ))
    }
    #[cfg(not(windows))]
    {
        run_with_time(context, plan, Path::new("/usr/bin/time"))
    }
}

#[cfg(not(windows))]
fn run_with_time(
    context: &RunContext,
    plan: &SuitePlan,
    time_binary: &Path,
) -> Result<SuiteOutput, SuiteError> {
    if !time_binary.is_file() {
        return Ok(SuiteOutput::skip_all(
            SuiteKind::Rss,
            "/usr/bin/time not found",
        ));
    }
    let mut cases = vec![(
        "rss.version".to_owned(),
        0,
        vec![path_arg(&context.skit), "--version".to_owned()],
    )];
    cases.extend(plan.library_sizes.iter().map(|n| {
        (
            format!("rss.list_json.n{n}"),
            *n,
            vec![
                path_arg(&context.skit),
                "list".to_owned(),
                "--json".to_owned(),
            ],
        )
    }));
    let mut output = SuiteOutput {
        suite: SuiteKind::Rss,
        duration_seconds: 0.0,
        metrics: BTreeMap::new(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    for (case, n, target) in cases {
        let mut peaks = Vec::with_capacity(plan.samples);
        for sample in 0..plan.samples {
            peaks.push(sample_peak(
                context,
                time_binary,
                n,
                &case,
                sample,
                &target,
            )?);
        }
        let (peak, maximum) = summarize_peaks(&peaks)?;
        output.metrics.insert(format!("{case}.peak_kib"), peak);
        output
            .metrics
            .insert(format!("{case}.peak_max_kib"), maximum);
        output
            .raw
            .insert(case, serde_json::json!({ "samples_kib": peaks }));
    }
    Ok(output)
}

fn summarize_peaks(peaks: &[u64]) -> Result<(Metric, Metric), SuiteError> {
    let values = peaks.iter().map(|value| *value as f64).collect::<Vec<_>>();
    let samples = values.len();
    let peak = Metric {
        value: median(&values)?,
        unit: "KiB".to_owned(),
        n: samples,
        p95: Some(nearest_rank_p95(&values)?),
        stddev: Some(sample_stddev(&values)?),
    };
    let maximum = Metric {
        value: values.iter().copied().fold(0.0, f64::max),
        unit: "KiB".to_owned(),
        n: samples,
        p95: None,
        stddev: None,
    };
    Ok((peak, maximum))
}

fn sample_peak(
    context: &RunContext,
    time_binary: &Path,
    n: usize,
    case: &str,
    sample: usize,
    target: &[String],
) -> Result<u64, SuiteError> {
    #[cfg(target_os = "macos")]
    {
        let mut argv = vec![path_arg(time_binary), "-l".to_owned()];
        argv.extend(target.iter().cloned());
        let output = run_process(&ProcessSpec {
            argv,
            cwd: context.workdir.clone(),
            env: context.environment(n)?,
            timeout: PROBE_TIMEOUT,
            check: true,
        })?;
        return Ok(bsd_time_max_kib(&String::from_utf8_lossy(&output.stderr))?);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let file = context
            .workdir
            .join(format!("{}_{}.rss", case.replace('.', "_"), sample));
        let mut argv = vec![
            path_arg(time_binary),
            "-f".to_owned(),
            "%M".to_owned(),
            "-o".to_owned(),
            file.display().to_string(),
            "--".to_owned(),
        ];
        argv.extend(target.iter().cloned());
        run_process(&ProcessSpec {
            argv,
            cwd: context.workdir.clone(),
            env: context.environment(n)?,
            timeout: PROBE_TIMEOUT,
            check: true,
        })?;
        let text = fs::read_to_string(&file).map_err(|error| {
            SuiteError::Contract(format!("could not read {}: {error}", file.display()))
        })?;
        Ok(gnu_time_max_kib(&text)?)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn peak_maximum_retains_the_number_of_independent_samples() {
        let (peak, maximum) = super::summarize_peaks(&[10, 30, 20]).unwrap();
        assert_eq!(peak.n, 3);
        assert_eq!(peak.value, 20.0);
        assert_eq!(maximum.n, 3);
        assert_eq!(maximum.value, 30.0);
        assert_eq!(maximum.p95, None);
        assert!(super::summarize_peaks(&[]).is_err());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn absent_time_and_missing_probe_output_are_typed() {
        use crate::{
            SuiteKind,
            suites::tests::{Fixture, executable, plan},
        };

        let fixture = Fixture::new();
        let missing = fixture.context.workdir.join("missing-time");
        let skipped =
            super::run_with_time(&fixture.context, &plan(SuiteKind::Rss, &[0]), &missing).unwrap();
        assert_eq!(skipped.skipped[0].reason, "/usr/bin/time not found");

        let silent = executable(
            &fixture.context.workdir.join("silent-time"),
            "#!/bin/sh\nexit 0\n",
        );
        assert!(
            super::sample_peak(
                &fixture.context,
                &silent,
                0,
                "missing.output",
                0,
                &[fixture.context.skit.display().to_string()]
            )
            .is_err()
        );
    }
}
