use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;

use crate::{
    Assembly, AssemblyError, Entry, EntryState, FormPlan, LaunchOptions, LaunchPlan,
    LaunchPlanError, ProgramResolver, ResolveError, assemble_delivery, build_launch_plan,
    plan_for_entry, resolve_values,
};

/// Frontend-supplied inputs for one headless run preparation.
#[derive(Debug, Clone, Copy)]
pub struct RunRequest<'a> {
    pub state: &'a EntryState,
    pub preset: Option<&'a str>,
    pub explicit: &'a BTreeMap<String, String>,
    pub extra_args: &'a [String],
    pub environment: &'a BTreeMap<String, String>,
    pub launch_options: &'a LaunchOptions,
}

/// A fully resolved run snapshot before the process boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRun {
    pub form: FormPlan,
    pub values: BTreeMap<String, String>,
    pub assembly: Assembly,
    pub launch: LaunchPlan,
    /// Same target/runtime/cwd as `launch`, but with secret-delivered argv/env values masked.
    pub masked_launch: LaunchPlan,
}

/// How one invocation chose its argv tail.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtraArgsResolution {
    pub args: Vec<String>,
    pub replayed: bool,
}

/// Named failures while preparing a headless run.
#[derive(Debug)]
pub enum PrepareRunError {
    UnknownPreset(String),
    Resolve(ResolveError),
    Assembly(AssemblyError),
    Launch(LaunchPlanError),
}

impl fmt::Display for PrepareRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPreset(name) => write!(formatter, "unknown preset: {name}"),
            Self::Resolve(source) => source.fmt(formatter),
            Self::Assembly(source) => source.fmt(formatter),
            Self::Launch(source) => source.fmt(formatter),
        }
    }
}

impl StdError for PrepareRunError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Resolve(source) => Some(source),
            Self::Assembly(source) => Some(source),
            Self::Launch(source) => Some(source),
            Self::UnknownPreset(_) => None,
        }
    }
}

impl From<ResolveError> for PrepareRunError {
    fn from(value: ResolveError) -> Self {
        Self::Resolve(value)
    }
}

impl From<AssemblyError> for PrepareRunError {
    fn from(value: AssemblyError) -> Self {
        Self::Assembly(value)
    }
}

impl From<LaunchPlanError> for PrepareRunError {
    fn from(value: LaunchPlanError) -> Self {
        Self::Launch(value)
    }
}

/// Prepare one non-interactive run through the shared form, assembly, and launch layers.
///
/// Resolution is `definition default < last-used < preset < explicit`. The named
/// preset must exist; an unknown explicit field is never silently ignored. The returned
/// launch snapshot is immutable and can be handed directly to `run_launch`. A parallel
/// masked snapshot is built from the same resolved runtime/cwd policy for transparency
/// and dry-run surfaces without exposing secret argv/env values.
///
/// # Errors
///
/// Returns a named error for an unknown preset, bad explicit key/value, missing secret
/// environment source, or launch preflight/runtime failure.
pub fn prepare_run(
    entry: &Entry,
    request: RunRequest<'_>,
    programs: &impl ProgramResolver,
) -> Result<PreparedRun, PrepareRunError> {
    if let Some(name) = request.preset
        && !request.state.presets.contains_key(name)
    {
        return Err(PrepareRunError::UnknownPreset(name.to_owned()));
    }
    let form = plan_for_entry(entry);
    let values = resolve_values(&form, request.state, request.preset, request.explicit)?;
    let assembly = assemble_delivery(&form, &values, request.extra_args, request.environment)?;
    let launch = build_launch_plan(entry, &assembly, request.launch_options, programs)?;
    let mut masked_assembly = assembly.clone();
    masked_assembly.args = assembly.masked_args.clone();
    masked_assembly.env_values = assembly.masked_env.clone();
    let masked_launch =
        build_launch_plan(entry, &masked_assembly, request.launch_options, programs)?;
    Ok(PreparedRun {
        form,
        values,
        assembly,
        launch,
        masked_launch,
    })
}

/// Build a launch snapshot that bypasses form values and remembered arguments entirely.
///
/// This is the raw execution contract: only the explicitly supplied argv tail participates.
/// The entry's target/runtime/dependency/workdir policy is still validated by the normal
/// launch planner.
///
/// # Errors
///
/// Returns the same target/runtime/workdir refusals as `build_launch_plan`.
pub fn prepare_raw_run(
    entry: &Entry,
    extra_args: &[String],
    launch_options: &LaunchOptions,
    programs: &impl ProgramResolver,
) -> Result<LaunchPlan, LaunchPlanError> {
    let assembly = Assembly {
        args: extra_args.to_vec(),
        masked_args: extra_args.to_vec(),
        ..Assembly::default()
    };
    build_launch_plan(entry, &assembly, launch_options, programs)
}

/// Resolve this run's remembered/supplied `--` tail.
///
/// Explicit args win. With none supplied, remembered args replay unless `forget` is
/// set. `replayed` lets frontends show the existing transparency note without
/// reimplementing the choice.
#[must_use]
pub fn resolve_extra_args(
    state: &EntryState,
    supplied: &[String],
    forget: bool,
) -> ExtraArgsResolution {
    if forget || !supplied.is_empty() {
        return ExtraArgsResolution {
            args: supplied.to_vec(),
            replayed: false,
        };
    }
    ExtraArgsResolution {
        args: state.extra_args.clone(),
        replayed: !state.extra_args.is_empty(),
    }
}

/// Project accepted raw values onto the last-used state contract.
///
/// Values equal to today's definition default are omitted so a later source/default
/// change is visible. Empty values are omitted unless the field deliberately delivers
/// an empty string. Secret stripping remains structural in `StateStore`.
#[must_use]
pub fn remembered_values(
    form: &FormPlan,
    values: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let by_key = form
        .fields
        .iter()
        .map(|field| (field.key.as_str(), field))
        .collect::<BTreeMap<_, _>>();
    values
        .iter()
        .filter_map(|(key, value)| {
            let field = by_key.get(key.as_str()).copied();
            if field
                .and_then(|field| field.default.as_deref())
                .is_some_and(|default| default == value)
            {
                return None;
            }
            if value.is_empty() && !field.is_some_and(|field| field.delivers_empty()) {
                return None;
            }
            Some((key.clone(), value.clone()))
        })
        .collect()
}
