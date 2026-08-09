//! CLI reads against the deterministic library-size grid.

use std::collections::BTreeMap;

use crate::{
    Skip, SuiteKind, SuiteOutput, SuitePlan,
    hyperfine::Case,
    runner::{RunContext, path_arg, run_hyperfine},
};

use super::SuiteError;

pub(super) fn run(context: &RunContext, plan: &SuitePlan) -> Result<SuiteOutput, SuiteError> {
    if context.hyperfine.is_none() {
        return Ok(SuiteOutput::skip_all(
            SuiteKind::Scale,
            "hyperfine not found",
        ));
    }
    let mut output = SuiteOutput {
        suite: SuiteKind::Scale,
        duration_seconds: 0.0,
        metrics: BTreeMap::new(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    let skit = path_arg(&context.skit);
    for n in &plan.library_sizes {
        let dataset = context.dataset(*n)?;
        let mut cases = vec![
            Case::new(
                format!("scale.list.n{n}"),
                [skit.clone(), "list".to_owned()],
            ),
            Case::new(
                format!("scale.list_json.n{n}"),
                [skit.clone(), "list".to_owned(), "--json".to_owned()],
            ),
        ];
        if *n > 0 {
            cases.push(Case::new(
                format!("scale.show.n{n}"),
                [
                    skit.clone(),
                    "show".to_owned(),
                    dataset.middle_slug()?.as_str().to_owned(),
                    "--json".to_owned(),
                ],
            ));
        }
        if plan.run_doctor {
            if context.uv.is_none() {
                output.skipped.push(Skip {
                    suite: SuiteKind::Scale,
                    case: format!("doctor_json.n{n}"),
                    reason: "uv not found".to_owned(),
                });
            } else {
                cases.push(Case::new(
                    format!("scale.doctor_json.n{n}"),
                    [skit.clone(), "doctor".to_owned(), "--json".to_owned()],
                ));
            }
        }
        let run = run_hyperfine(
            context,
            plan,
            &cases,
            context.environment(*n)?,
            &format!("scale_n{n}"),
        )?;
        for (name, metric) in run.metrics {
            if output.metrics.insert(name.clone(), metric).is_some() {
                return Err(SuiteError::Contract(format!(
                    "duplicate scale metric {name}"
                )));
            }
        }
        output.raw.insert(
            format!("n{n}"),
            serde_json::Value::Object(run.raw.into_iter().collect()),
        );
    }
    Ok(output)
}
