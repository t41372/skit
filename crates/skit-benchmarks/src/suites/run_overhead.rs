//! Launch overhead relative to each language runtime.

use std::collections::BTreeMap;

use skit_application::EntryRepository as _;
use skit_store::FileStore;

use crate::{
    Skip, SuiteKind, SuiteOutput, SuitePlan,
    dataset::{dataset_dirs, generate_runover},
    hyperfine::Case,
    runner::{RunContext, path_arg, run_hyperfine},
};

use super::SuiteError;

pub(super) fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    if context.hyperfine.is_none() {
        return Ok(SuiteOutput::skip_all(
            SuiteKind::RunOverhead,
            "hyperfine not found",
        ));
    }
    let Some(uv) = &context.uv else {
        return Ok(SuiteOutput::skip_all(
            SuiteKind::RunOverhead,
            "uv not found",
        ));
    };
    let Some(python) = &context.python else {
        return Ok(SuiteOutput::skip_all(
            SuiteKind::RunOverhead,
            "python not found",
        ));
    };
    let manifest = generate_runover(context.workdir.join("runover"))?;
    let dirs = dataset_dirs(&manifest.root)?;
    let store = FileStore::new(dirs.data);
    let python_entry = store
        .resolve(manifest.slugs[0].as_str())
        .map_err(|error| SuiteError::Contract(error.to_string()))?;
    let shell_entry = store
        .resolve(manifest.slugs[1].as_str())
        .map_err(|error| SuiteError::Contract(error.to_string()))?;
    let js_entry = store
        .resolve(manifest.slugs[2].as_str())
        .map_err(|error| SuiteError::Contract(error.to_string()))?;
    let python_source = store
        .payload_path(&python_entry)
        .map_err(|error| SuiteError::Contract(error.to_string()))?;
    let shell_source = store
        .payload_path(&shell_entry)
        .map_err(|error| SuiteError::Contract(error.to_string()))?;
    let js_source = store
        .payload_path(&js_entry)
        .map_err(|error| SuiteError::Contract(error.to_string()))?;
    let skit = path_arg(&context.skit);
    let mut output = SuiteOutput {
        suite: SuiteKind::RunOverhead,
        duration_seconds: 0.0,
        metrics: BTreeMap::new(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    let mut cases = vec![
        Case::new(
            "run_overhead.python.python",
            [path_arg(python), path_arg(&python_source)],
        ),
        Case::new(
            "run_overhead.python.uv_script",
            [
                path_arg(uv),
                "run".to_owned(),
                "--no-project".to_owned(),
                "--script".to_owned(),
                path_arg(&python_source),
            ],
        ),
        Case::new(
            "run_overhead.python.skit",
            [
                skit.clone(),
                "run".to_owned(),
                manifest.slugs[0].as_str().to_owned(),
                "--no-input".to_owned(),
            ],
        ),
    ];
    if let Some(bash) = &context.bash {
        cases.extend([
            Case::new(
                "run_overhead.shell.bash",
                [path_arg(bash), path_arg(&shell_source)],
            ),
            Case::new(
                "run_overhead.shell.skit",
                [
                    skit.clone(),
                    "run".to_owned(),
                    manifest.slugs[1].as_str().to_owned(),
                    "--no-input".to_owned(),
                ],
            ),
        ]);
    } else {
        output.skipped.push(Skip {
            suite: SuiteKind::RunOverhead,
            case: "shell".to_owned(),
            reason: "bash not found".to_owned(),
        });
    }
    if plan.run_javascript_lane {
        if let Some(node) = &context.node {
            cases.extend([
                Case::new(
                    "run_overhead.js.node",
                    [path_arg(node), path_arg(&js_source)],
                ),
                Case::new(
                    "run_overhead.js.skit",
                    [
                        skit,
                        "run".to_owned(),
                        manifest.slugs[2].as_str().to_owned(),
                        "--no-input".to_owned(),
                    ],
                ),
            ]);
        } else {
            output.skipped.push(Skip {
                suite: SuiteKind::RunOverhead,
                case: "js".to_owned(),
                reason: "node not found".to_owned(),
            });
        }
    }
    let run = run_hyperfine(
        context,
        plan,
        &cases,
        context.environment_for(&manifest.root)?,
        "run_overhead",
    )?;
    output.metrics = run.metrics;
    output.raw = run.raw;
    Ok(output)
}
