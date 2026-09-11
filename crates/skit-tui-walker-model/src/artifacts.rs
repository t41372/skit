use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    str::FromStr,
};

/// Name of the cast inside one failure bundle.
pub const FAILURE_CAST_NAME: &str = "failure.cast";

/// Name of the cast inside one success bundle.
pub const SUCCESS_CAST_NAME: &str = "success.cast";

/// Name of the replay description inside one bundle.
pub const REPRO_NAME: &str = "repro.json";

/// Environment variable that sets the number of random cases.
pub const CASES_VARIABLE: &str = "SKIT_WALKER_CASES";

/// Environment variable that sets the number of operations in one case.
pub const STEPS_VARIABLE: &str = "SKIT_WALKER_STEPS";

/// Environment variable that selects the profile matrix.
pub const PROFILES_VARIABLE: &str = "SKIT_WALKER_PROFILES";

/// Environment variable that requests a cast of one successful walk.
pub const RECORD_SUCCESS_VARIABLE: &str = "SKIT_WALKER_RECORD_SUCCESS";

/// Environment variable that sets the liveness sampling interval.
pub const LIVENESS_EVERY_VARIABLE: &str = "SKIT_WALKER_LIVENESS_EVERY";

/// Environment variable that names one stored replay.
pub const REPRO_VARIABLE: &str = "SKIT_WALKER_REPRO";

const PROFILE_VALUES: &[&str] = &["bounded", "complete"];
const PROFILE_MODES: &[ProfileMode] = &[ProfileMode::Bounded, ProfileMode::Complete];
const RECORD_SUCCESS_VALUES: &[&str] = &["0", "1"];
const RECORD_SUCCESS_MODES: &[SuccessRecording] =
    &[SuccessRecording::Skip, SuccessRecording::Record];

/// Which profile matrix one walk replays each trace against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileMode {
    /// One profile.
    Bounded,
    /// Every profile of the matrix.
    Complete,
}

/// Whether one walk keeps a cast of a successful case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuccessRecording {
    /// Keep no cast of a successful case.
    Skip,
    /// Keep one cast of a successful case.
    Record,
}

/// How often one walk runs liveness inside a case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LivenessSampling {
    /// Run liveness at the end of the case only.
    ///
    /// `SKIT_WALKER_LIVENESS_EVERY=0` selects this value.
    Never,
    /// Run liveness after this many operations.
    Every(NonZeroUsize),
}

/// Where one walk takes its operations from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReproSource {
    /// Draw a new trace.
    Fresh,
    /// Replay the trace in this file.
    Replay(PathBuf),
}

/// Write one failure bundle below `directory` and return its path.
///
/// The bundle holds `repro.json` and, for a nonempty cast, `failure.cast`. Two failures never
/// share one bundle.
pub fn write_failure_bundle(
    directory: &Path,
    repro: &serde_json::Value,
    cast: &[u8],
) -> Result<PathBuf, String> {
    write_bundle(directory, "failure", FAILURE_CAST_NAME, repro, cast)
}

/// Write one success bundle below `directory` and return its path.
///
/// The bundle holds `repro.json` and, for a nonempty cast, `success.cast`.
pub fn write_success_bundle(
    directory: &Path,
    repro: &serde_json::Value,
    cast: &[u8],
) -> Result<PathBuf, String> {
    write_bundle(directory, "success", SUCCESS_CAST_NAME, repro, cast)
}

fn write_bundle(
    directory: &Path,
    kind: &str,
    cast_name: &str,
    repro: &serde_json::Value,
    cast: &[u8],
) -> Result<PathBuf, String> {
    fs::create_dir_all(directory).map_err(|failure| failure.to_string())?;
    let staged_prefix = format!(".{kind}-");
    let staged = tempfile::Builder::new()
        .prefix(&staged_prefix)
        .tempdir_in(directory)
        .map_err(|failure| failure.to_string())?;
    let encoded = serde_json::to_vec_pretty(repro).map_err(|failure| failure.to_string())?;
    if !cast.is_empty() {
        fs::write(staged.path().join(cast_name), cast).map_err(|failure| failure.to_string())?;
    }
    fs::write(staged.path().join(REPRO_NAME), encoded).map_err(|failure| failure.to_string())?;
    let staged_path = staged.keep();
    let bundle_name = staged_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix('.'))
        .ok_or_else(|| format!("invalid staged bundle path: {}", staged_path.display()))?;
    let final_path = directory.join(bundle_name);
    publish_bundle(&staged_path, &final_path)?;
    Ok(final_path)
}

/// Rename one staged bundle into its final place.
///
/// A failed rename removes the staged directory. The artifact directory then keeps no partial
/// bundle. The cleanup is best effort and it keeps the error of the rename.
pub(crate) fn publish_bundle(staged: &Path, destination: &Path) -> Result<(), String> {
    let Err(failure) = fs::rename(staged, destination) else {
        return Ok(());
    };
    let _ = fs::remove_dir_all(staged);
    Err(failure.to_string())
}

/// Read the number of random cases in one walk.
///
/// A present value must be a positive integer, or the reader stops the walk.
#[must_use]
pub fn walk_cases(default: u32) -> u32 {
    positive(
        std::env::var(CASES_VARIABLE).ok(),
        default,
        "SKIT_WALKER_CASES must be a positive integer",
    )
}

/// Read the number of operations in one case.
///
/// A present value must be a positive integer, or the reader stops the walk.
#[must_use]
pub fn walk_steps(default: usize) -> usize {
    positive(
        std::env::var(STEPS_VARIABLE).ok(),
        default,
        "SKIT_WALKER_STEPS must be a positive integer",
    )
}

/// Read which profile matrix one walk replays each trace against.
#[must_use]
pub fn walk_profiles() -> ProfileMode {
    PROFILE_MODES[selected(
        PROFILE_VALUES,
        std::env::var(PROFILES_VARIABLE).ok(),
        "SKIT_WALKER_PROFILES must be bounded or complete",
    )]
}

/// Read whether one walk keeps a cast of a successful case.
#[must_use]
pub fn record_success() -> SuccessRecording {
    RECORD_SUCCESS_MODES[selected(
        RECORD_SUCCESS_VALUES,
        std::env::var(RECORD_SUCCESS_VARIABLE).ok(),
        "SKIT_WALKER_RECORD_SUCCESS must be 0 or 1",
    )]
}

/// Read how often one walk runs liveness inside a case.
///
/// The value `0` selects [`LivenessSampling::Never`]. Another present value must be a positive
/// integer, or the reader stops the walk.
#[must_use]
pub fn liveness_every() -> LivenessSampling {
    sampling(std::env::var(LIVENESS_EVERY_VARIABLE).ok())
}

/// Read where one walk takes its operations from.
#[must_use]
pub fn repro_source() -> ReproSource {
    replay(std::env::var(REPRO_VARIABLE).ok())
}

pub(crate) fn positive<T>(raw: Option<String>, default: T, requirement: &str) -> T
where
    T: Copy + Default + FromStr + PartialOrd,
{
    let value = raw.map_or(default, |raw| {
        raw.parse::<T>()
            .ok()
            .filter(|value| *value > T::default())
            .unwrap_or_else(|| panic!("{requirement}, not {raw:?}"))
    });
    // The default is a code constant. A zero default is a defect of the caller, not of the user.
    assert!(value > T::default(), "{requirement}");
    value
}

pub(crate) fn selected(values: &[&str], raw: Option<String>, requirement: &str) -> usize {
    let raw = raw.unwrap_or_else(|| values[0].to_owned());
    values
        .iter()
        .position(|value| *value == raw)
        .expect(requirement)
}

pub(crate) fn sampling(raw: Option<String>) -> LivenessSampling {
    raw.map_or(LivenessSampling::Never, |raw| {
        let interval = raw.parse::<usize>().unwrap_or_else(|_| {
            panic!("SKIT_WALKER_LIVENESS_EVERY must be zero or a positive integer, not {raw:?}")
        });
        NonZeroUsize::new(interval).map_or(LivenessSampling::Never, LivenessSampling::Every)
    })
}

pub(crate) fn replay(raw: Option<String>) -> ReproSource {
    raw.map_or(ReproSource::Fresh, |raw| {
        ReproSource::Replay(PathBuf::from(raw))
    })
}
