//! Stable statistical definitions used by every benchmark suite.

use statrs::statistics::{Data, OrderStatistics as _, Statistics as _};
use thiserror::Error;

/// A statistical summary could not be computed.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum StatsError {
    /// The sample set was empty.
    #[error("cannot summarize an empty sample set")]
    Empty,
    /// A sample was NaN or infinite.
    #[error("samples must be finite")]
    NonFinite,
}

fn sorted(values: &[f64]) -> Result<Vec<f64>, StatsError> {
    if values.is_empty() {
        return Err(StatsError::Empty);
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(StatsError::NonFinite);
    }
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    Ok(values)
}

/// Return the middle sample or mean of the two middle samples.
pub fn median(values: &[f64]) -> Result<f64, StatsError> {
    let values = sorted(values)?;
    Ok(Data::new(values).median())
}

/// Return the nearest-rank 95th percentile (`ceil(0.95 * n)`).
pub fn nearest_rank_p95(values: &[f64]) -> Result<f64, StatsError> {
    let values = sorted(values)?;
    let rank = (95 * values.len()).div_ceil(100);
    Ok(values[rank - 1])
}

/// Return the sample standard deviation. One sample has zero spread.
pub fn sample_stddev(values: &[f64]) -> Result<f64, StatsError> {
    if values.is_empty() {
        return Err(StatsError::Empty);
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(StatsError::NonFinite);
    }
    if values.len() == 1 {
        return Ok(0.0);
    }
    Ok(values.std_dev())
}
