//! In-process store, analyzer, launch-plan, and prompt-render measurements.

use std::{
    collections::BTreeMap,
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use skit_application::{
    EntryRepository as _, LibraryService,
    delivery::{PreparedValue, assemble},
    form_state::FormStateService,
};
use skit_domain::{Entry, EntrySettings, parameters::ParameterType};
use skit_language::{detect_candidates, render_prompt_body};
use skit_runtime::{LaunchPaths, SystemProbe, build_launch_preview};
use skit_store::{FileFormStateStore, FileStore};

use crate::{
    Metric, Skip, SuiteKind, SuiteOutput, SuitePlan,
    dataset::dataset_dirs,
    process::{ProcessSpec, run as run_process},
    runner::{PROBE_TIMEOUT, RunContext, path_arg},
    sources::{LANGUAGES, extension, generate, generate_broken},
    stats::{median, nearest_rank_p95, sample_stddev},
};

use super::SuiteError;

const SOURCE_LINES: &[usize] = &[20, 200, 2_000];
const BROKEN_LINES: usize = 2_000;
const COLD_SAMPLES: usize = 5;
const MAX_ITERATIONS: usize = 1 << 24;

pub(super) fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    let mut output = SuiteOutput {
        suite: SuiteKind::Micro,
        duration_seconds: 0.0,
        metrics: BTreeMap::new(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    let (sample_count, target) = measurement_config(plan.fast);
    measure_store(context, plan, sample_count, target, &mut output)?;
    let sources = materialize_sources(context)?;
    measure_analyzers(context, plan, &sources, sample_count, target, &mut output)?;
    measure_launch(context, plan, sample_count, target, &mut output)?;
    measure_prompt(sample_count, target, &mut output)?;
    Ok(output)
}

fn measurement_config(fast: bool) -> (usize, Duration) {
    if fast {
        (10, Duration::from_millis(10))
    } else {
        (20, Duration::from_millis(30))
    }
}

fn measure_store(
    context: &RunContext,
    plan: &SuitePlan,
    samples: usize,
    target: Duration,
    output: &mut SuiteOutput,
) -> Result<(), SuiteError> {
    for n in &plan.library_sizes {
        let dirs = dataset_dirs(&context.dataset(*n)?.root)?;
        let store = FileStore::new(dirs.data);
        let state = FormStateService::new(FileFormStateStore::new(dirs.state));
        record(
            output,
            &format!("micro.store.list_entries.n{n}.median_us"),
            measure(samples, target, || {
                let scan = store
                    .scan()
                    .map_err(|error| SuiteError::Contract(error.to_string()))?;
                let mut entries = Vec::with_capacity(scan.entries.len());
                for summary in scan.entries {
                    entries.push(
                        store
                            .resolve(summary.slug.as_str())
                            .map_err(|error| SuiteError::Contract(error.to_string()))?,
                    );
                }
                black_box(entries);
                Ok(())
            })?,
            &format!("store_n{n}"),
            &format!("store.list_entries.n{n}"),
        )?;
        record(
            output,
            &format!("micro.store.list_summaries.n{n}.median_us"),
            measure(samples, target, || {
                black_box(
                    LibraryService::new(store.clone())
                        .list()
                        .map_err(|error| SuiteError::Contract(error.to_string()))?,
                );
                Ok(())
            })?,
            &format!("store_n{n}"),
            &format!("store.list_summaries.n{n}"),
        )?;
        if let Some(slug) = context.dataset(*n)?.slugs.get(n / 2).cloned() {
            record(
                output,
                &format!("micro.store.resolve.n{n}.median_us"),
                measure(samples, target, || {
                    black_box(
                        store
                            .resolve(slug.as_str())
                            .map_err(|error| SuiteError::Contract(error.to_string()))?,
                    );
                    Ok(())
                })?,
                &format!("store_n{n}"),
                &format!("store.resolve.n{n}"),
            )?;
            record(
                output,
                &format!("micro.argstate.load_state.n{n}.median_us"),
                measure(samples, target, || {
                    black_box(state.load(&slug));
                    Ok(())
                })?,
                &format!("store_n{n}"),
                &format!("argstate.load_state.n{n}"),
            )?;
        }
    }
    Ok(())
}

fn materialize_sources(context: &RunContext) -> Result<BTreeMap<String, PathBuf>, SuiteError> {
    let directory = context.workdir.join("sources");
    fs::create_dir_all(&directory).map_err(|error| {
        SuiteError::Contract(format!("could not create {}: {error}", directory.display()))
    })?;
    let mut paths = BTreeMap::new();
    for language in LANGUAGES {
        for lines in SOURCE_LINES {
            let path = directory.join(format!("{language}_{lines}.{}", extension(language)));
            write_source(&path, &generate(language, *lines).map_err(source_error)?)?;
            paths.insert(format!("{language}:{lines}:valid"), path);
        }
        let broken = directory.join(format!(
            "{language}_{BROKEN_LINES}_broken.{}",
            extension(language)
        ));
        write_source(
            &broken,
            &generate_broken(language, BROKEN_LINES).map_err(source_error)?,
        )?;
        paths.insert(format!("{language}:{BROKEN_LINES}:broken"), broken);
    }
    Ok(paths)
}

fn measure_analyzers(
    context: &RunContext,
    plan: &SuitePlan,
    sources: &BTreeMap<String, PathBuf>,
    samples: usize,
    target: Duration,
    output: &mut SuiteOutput,
) -> Result<(), SuiteError> {
    for language in LANGUAGES {
        for lines in SOURCE_LINES {
            let text = read_source(&sources[&format!("{language}:{lines}:valid")])?;
            record(
                output,
                &format!("micro.analyze.{language}.l{lines}.median_us"),
                measure(samples, target, || {
                    black_box(detect_candidates(language, black_box(&text)));
                    Ok(())
                })?,
                "analyzers",
                &format!("analyze.{language}.l{lines}"),
            )?;
        }
        let broken_path = &sources[&format!("{language}:{BROKEN_LINES}:broken")];
        let broken = read_source(broken_path)?;
        record(
            output,
            &format!("micro.analyze_broken.{language}.l{BROKEN_LINES}.median_us"),
            measure(samples, target, || {
                black_box(detect_candidates(language, black_box(&broken)));
                Ok(())
            })?,
            "analyzers",
            &format!("analyze_broken.{language}.l{BROKEN_LINES}"),
        )?;
        cold_analyzer(
            context,
            plan,
            language,
            &sources[&format!("{language}:200:valid")],
            output,
        )?;
    }
    Ok(())
}

fn cold_analyzer(
    context: &RunContext,
    plan: &SuitePlan,
    language: &str,
    source: &Path,
    output: &mut SuiteOutput,
) -> Result<(), SuiteError> {
    let mut samples = Vec::with_capacity(COLD_SAMPLES);
    for _ in 0..COLD_SAMPLES {
        let result = run_process(&ProcessSpec {
            argv: vec![
                path_arg(&context.harness),
                "probe".to_owned(),
                "analyze".to_owned(),
                "--kind".to_owned(),
                language.to_owned(),
                "--source".to_owned(),
                path_arg(source),
            ],
            cwd: context.workdir.clone(),
            env: context.environment(0)?,
            timeout: PROBE_TIMEOUT,
            check: true,
        });
        let result = match result {
            Ok(result) => result,
            Err(error) if plan.compare_mode => {
                output.skipped.push(Skip {
                    suite: SuiteKind::Micro,
                    case: format!("analyze_cold.{language}"),
                    reason: error.to_string(),
                });
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let text = String::from_utf8_lossy(&result.stdout);
        let duration = parse_probe_duration(&text).map_err(|reason| {
            SuiteError::Contract(format!(
                "cold analyzer probe for {language} returned invalid timing: {reason}"
            ))
        })?;
        samples.push(duration);
    }
    let metric_id = format!("micro.analyze_cold.{language}.median_ms");
    insert_metric(output, &metric_id, &samples, "ms")?;
    let cold = output
        .raw
        .entry("analyze_cold".to_owned())
        .or_insert_with(|| serde_json::json!({}));
    let serde_json::Value::Object(cold) = cold else {
        return Err(SuiteError::Contract(
            "micro raw analyze_cold payload is not an object".to_owned(),
        ));
    };
    if cold
        .insert(
            language.to_owned(),
            serde_json::json!({ "samples_ms": samples }),
        )
        .is_some()
    {
        return Err(SuiteError::Contract(format!(
            "duplicate micro raw series analyze_cold.{language}"
        )));
    }
    Ok(())
}

fn parse_probe_duration(text: &str) -> Result<f64, String> {
    let value = text
        .trim()
        .parse::<f64>()
        .map_err(|error| format!("{text:?}: {error}"))?;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(format!("expected a finite non-negative value, got {value}"))
    }
}

fn measure_launch(
    context: &RunContext,
    plan: &SuitePlan,
    samples: usize,
    target: Duration,
    output: &mut SuiteOutput,
) -> Result<(), SuiteError> {
    let n = *plan
        .library_sizes
        .iter()
        .max()
        .ok_or_else(|| SuiteError::Contract("micro launch needs a dataset".to_owned()))?;
    let manifest = context.dataset(n)?;
    let dirs = dataset_dirs(&manifest.root)?;
    let store = FileStore::new(dirs.data);
    for kind in ["python", "shell", "command"] {
        let Some(slug) = manifest
            .kinds
            .iter()
            .find_map(|(slug, found)| (found == kind).then_some(slug))
        else {
            output.skipped.push(Skip {
                suite: SuiteKind::Micro,
                case: format!("launch.describe.{kind}"),
                reason: format!("dataset n{n} has no {kind} entry"),
            });
            continue;
        };
        let entry = store
            .resolve(slug)
            .map_err(|error| SuiteError::Contract(error.to_string()))?;
        let paths = launch_paths(context, &store, &entry)?;
        record(
            output,
            &format!("micro.launch.describe.{kind}.median_us"),
            measure(samples, target, || preview(&entry, &paths))?,
            "launch",
            &format!("launch.describe.{kind}"),
        )?;
    }
    Ok(())
}

fn launch_paths(
    context: &RunContext,
    store: &FileStore,
    entry: &Entry,
) -> Result<LaunchPaths, SuiteError> {
    let script = if entry.meta.kind.as_str() == "command" {
        context.workdir.join("unused-command-source")
    } else {
        store
            .payload_path(entry)
            .map_err(|error| SuiteError::Contract(error.to_string()))?
    };
    Ok(LaunchPaths {
        script,
        entry_dir: store.entry_dir_path(&entry.slug),
        invoke_cwd: context.workdir.clone(),
    })
}

fn preview(entry: &Entry, paths: &LaunchPaths) -> Result<(), SuiteError> {
    let settings = EntrySettings::from_meta(&entry.meta);
    let values = settings
        .parameters
        .iter()
        .map(|declaration| {
            let value = if declaration.parameter_type == ParameterType::Bool {
                "true"
            } else {
                "value"
            };
            (
                declaration.name.clone(),
                PreparedValue::Scalar(value.to_owned()),
            )
        })
        .collect();
    let assembly = assemble(&settings.parameters, &values, &[])
        .map_err(|error| SuiteError::Contract(error.to_string()))?;
    black_box(
        build_launch_preview(entry, paths, &assembly, None, None, None, &SystemProbe)
            .map_err(|error| SuiteError::Contract(error.to_string()))?,
    );
    Ok(())
}

fn measure_prompt(
    samples: usize,
    target: Duration,
    output: &mut SuiteOutput,
) -> Result<(), SuiteError> {
    let body = "Review the repository at {{path}} and produce a report about {{topic}}.\n\
                Constraints: keep it under {{limit}} words, cite files by path, and\n\
                prefer bullet lists. Repeat the summary at the end.\n"
        .repeat(10);
    let values = BTreeMap::from([
        ("path".to_owned(), "/workspace/repo".to_owned()),
        ("topic".to_owned(), "error handling".to_owned()),
        ("limit".to_owned(), "500".to_owned()),
    ]);
    record(
        output,
        "micro.prompt.render_body.median_us",
        measure(samples, target, || {
            black_box(render_prompt_body(&body, &values, true));
            Ok(())
        })?,
        "render",
        "prompt.render_body",
    )
}

fn measure(
    sample_count: usize,
    target: Duration,
    mut operation: impl FnMut() -> Result<(), SuiteError>,
) -> Result<Vec<f64>, SuiteError> {
    if sample_count == 0 {
        return Err(SuiteError::Contract(
            "micro sample count must be positive".to_owned(),
        ));
    }
    for _ in 0..3 {
        operation()?;
    }
    let mut iterations = 1_usize;
    loop {
        let started = Instant::now();
        for _ in 0..iterations {
            operation()?;
        }
        if started.elapsed() >= target || iterations >= MAX_ITERATIONS {
            break;
        }
        iterations = iterations.saturating_mul(2).min(MAX_ITERATIONS);
    }
    let mut samples = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        let started = Instant::now();
        for _ in 0..iterations {
            operation()?;
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64);
    }
    Ok(samples)
}

fn record(
    output: &mut SuiteOutput,
    metric_id: &str,
    samples: Vec<f64>,
    raw_group: &str,
    raw_case: &str,
) -> Result<(), SuiteError> {
    insert_metric(output, metric_id, &samples, "us")?;
    let raw_samples = samples
        .iter()
        .map(|value| value / 1_000_000.0)
        .collect::<Vec<_>>();
    let group = output
        .raw
        .entry(raw_group.to_owned())
        .or_insert_with(|| serde_json::json!({}));
    let serde_json::Value::Object(group) = group else {
        return Err(SuiteError::Contract(format!(
            "micro raw group {raw_group} is not an object"
        )));
    };
    if group
        .insert(raw_case.to_owned(), serde_json::json!(raw_samples))
        .is_some()
    {
        return Err(SuiteError::Contract(format!(
            "duplicate micro raw series {raw_group}.{raw_case}"
        )));
    }
    Ok(())
}

fn insert_metric(
    output: &mut SuiteOutput,
    metric_id: &str,
    samples: &[f64],
    unit: &str,
) -> Result<(), SuiteError> {
    let metric = Metric {
        value: median(samples)?,
        unit: unit.to_owned(),
        n: samples.len(),
        p95: Some(nearest_rank_p95(samples)?),
        stddev: Some(sample_stddev(samples)?),
    };
    if output
        .metrics
        .insert(metric_id.to_owned(), metric)
        .is_some()
    {
        return Err(SuiteError::Contract(format!(
            "duplicate micro metric {metric_id}"
        )));
    }
    Ok(())
}

fn write_source(path: &Path, text: &str) -> Result<(), SuiteError> {
    fs::write(path, text).map_err(|error| {
        SuiteError::Contract(format!("could not write {}: {error}", path.display()))
    })
}

fn read_source(path: &Path) -> Result<String, SuiteError> {
    fs::read_to_string(path).map_err(|error| {
        SuiteError::Contract(format!("could not read {}: {error}", path.display()))
    })
}

fn source_error(error: crate::sources::SourceError) -> SuiteError {
    SuiteError::Contract(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{SuiteKind, SuiteOutput};

    fn output() -> SuiteOutput {
        SuiteOutput {
            suite: SuiteKind::Micro,
            duration_seconds: 0.0,
            metrics: BTreeMap::new(),
            skipped: Vec::new(),
            raw: BTreeMap::new(),
        }
    }

    #[test]
    fn adaptive_measurements_return_finite_per_call_samples() {
        let samples = super::measure(3, std::time::Duration::from_millis(1), || {
            std::hint::black_box(2_u64.wrapping_mul(3));
            Ok(())
        })
        .unwrap();
        assert_eq!(samples.len(), 3);
        assert!(
            samples
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
        );
        assert_eq!(super::measurement_config(true).0, 10);
        assert_eq!(super::measurement_config(false).0, 20);
        assert_eq!(
            super::measurement_config(false).1,
            std::time::Duration::from_millis(30)
        );
        assert!(super::measure(0, std::time::Duration::ZERO, || Ok(())).is_err());
        assert!(
            super::measure(1, std::time::Duration::ZERO, || {
                Err(super::SuiteError::Contract("operation failed".to_owned()))
            })
            .is_err()
        );
    }

    #[test]
    fn cold_probe_timing_must_be_one_finite_non_negative_number() {
        assert_eq!(super::parse_probe_duration("12.5\n").unwrap(), 12.5);
        assert!(super::parse_probe_duration("NaN").is_err());
        assert!(super::parse_probe_duration("-1").is_err());
        assert!(super::parse_probe_duration("12\n13").is_err());
    }

    #[test]
    fn warm_raw_samples_keep_latest_main_groups_names_and_seconds() {
        let mut recorded = output();
        super::record(
            &mut recorded,
            "micro.analyze.python.l20.median_us",
            vec![10.0, 20.0],
            "analyzers",
            "analyze.python.l20",
        )
        .unwrap();

        assert_eq!(
            recorded.raw["analyzers"]["analyze.python.l20"],
            serde_json::json!([0.00001, 0.00002])
        );
        assert!(
            !recorded
                .raw
                .contains_key("micro.analyze.python.l20.median_us")
        );

        recorded
            .raw
            .insert("broken".to_owned(), serde_json::json!([]));
        assert!(
            super::record(
                &mut recorded,
                "micro.broken.median_us",
                vec![1.0],
                "broken",
                "case"
            )
            .is_err()
        );
        assert!(
            super::record(
                &mut recorded,
                "micro.analyze.python.l20.median_us",
                vec![1.0],
                "analyzers",
                "another"
            )
            .is_err()
        );

        let mut duplicate_raw = output();
        duplicate_raw.raw.insert(
            "group".to_owned(),
            serde_json::json!({"case": [0.1]}),
        );
        assert!(
            super::record(
                &mut duplicate_raw,
                "micro.new.median_us",
                vec![1.0],
                "group",
                "case"
            )
            .is_err()
        );
    }

    #[test]
    fn source_io_errors_remain_suite_contract_errors() {
        let root = tempfile::TempDir::new().unwrap();
        let missing = root.path().join("missing/source");
        assert!(super::write_source(&missing, "body").is_err());
        assert!(super::read_source(&missing).is_err());
        assert!(
            super::source_error(crate::sources::SourceError::TooShort)
                .to_string()
                .contains("lines")
        );
    }

    #[cfg(unix)]
    #[test]
    fn cold_and_launch_probes_cover_compare_and_corrupt_payload_paths() {
        use crate::suites::tests::{Fixture, executable, plan};

        let mut fixture = Fixture::new();
        let source = fixture.context.workdir.join("source.py");
        std::fs::write(&source, "print('ok')\n").unwrap();
        let failing = executable(
            &fixture.context.workdir.join("failing-probe"),
            "#!/bin/sh\nexit 7\n",
        );
        fixture.context.harness = failing;
        let mut compare = plan(SuiteKind::Micro, &[0]);
        compare.compare_mode = true;
        let mut measured = output();
        super::cold_analyzer(&fixture.context, &compare, "python", &source, &mut measured).unwrap();
        assert_eq!(measured.skipped[0].case, "analyze_cold.python");

        let strict = plan(SuiteKind::Micro, &[0]);
        assert!(
            super::cold_analyzer(&fixture.context, &strict, "python", &source, &mut output())
                .is_err()
        );

        fixture.context.harness = executable(
            &fixture.context.workdir.join("invalid-probe"),
            "#!/bin/sh\nprintf 'invalid\\n'\n",
        );
        assert!(
            super::cold_analyzer(&fixture.context, &strict, "python", &source, &mut output())
                .is_err()
        );

        fixture.context.harness = executable(
            &fixture.context.workdir.join("valid-probe"),
            "#!/bin/sh\nprintf '1.0\\n'\n",
        );
        let mut wrong_raw = output();
        wrong_raw
            .raw
            .insert("analyze_cold".to_owned(), serde_json::json!([]));
        assert!(
            super::cold_analyzer(&fixture.context, &strict, "python", &source, &mut wrong_raw)
                .is_err()
        );
        let mut duplicate = output();
        duplicate.raw.insert(
            "analyze_cold".to_owned(),
            serde_json::json!({"python": {"samples_ms": [1.0]}}),
        );
        assert!(
            super::cold_analyzer(&fixture.context, &strict, "python", &source, &mut duplicate)
                .is_err()
        );

        let mut missing_kinds = output();
        super::measure_launch(
            &fixture.context,
            &strict,
            1,
            std::time::Duration::ZERO,
            &mut missing_kinds,
        )
        .unwrap();
        assert_eq!(missing_kinds.skipped.len(), 3);
    }
}
