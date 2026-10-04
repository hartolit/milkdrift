use std::time::Duration;

use serde::Serialize;

/// Harness error preserving the concrete owning failure.
pub type EvidenceError = Box<dyn std::error::Error + Send + Sync>;

/// Result type used by every evidence scenario.
pub type EvidenceResult<T = ()> = Result<T, EvidenceError>;

/// One deterministic scenario result used to keep benchmark work observable.
#[derive(Clone, Debug, Serialize)]
pub struct ScenarioMeasurement {
    /// Stable scenario label.
    pub scenario: &'static str,
    /// Logical operations performed.
    pub operations: u64,
    /// Bytes processed when meaningful.
    pub bytes: u64,
    /// Stable checksum preventing dead-code elimination of the measured result.
    pub checksum: String,
}

impl ScenarioMeasurement {
    /// Constructs a measurement from exact result bytes.
    #[must_use]
    pub fn new(scenario: &'static str, operations: u64, bytes: u64, evidence: &[u8]) -> Self {
        Self {
            scenario,
            operations,
            bytes,
            checksum: format!("b3_{}", blake3::hash(evidence)),
        }
    }
}

/// Sorted latency distribution recorded without making it a correctness budget.
#[derive(Clone, Debug, Serialize)]
pub struct LatencySummary {
    /// Number of observed requests.
    pub count: u64,
    /// Minimum latency in microseconds.
    pub minimum_us: u64,
    /// Median latency in microseconds.
    pub p50_us: u64,
    /// 95th percentile latency in microseconds.
    pub p95_us: u64,
    /// 99th percentile latency in microseconds.
    pub p99_us: u64,
    /// Maximum latency in microseconds.
    pub maximum_us: u64,
}

impl LatencySummary {
    /// Summarizes one non-empty sample using nearest-rank indices.
    ///
    /// # Errors
    /// Refuses an empty sample or durations that cannot be expressed in unsigned microseconds.
    /// An unrepresentable count or percentile index also fails instead of truncating the report.
    pub fn from_durations(mut samples: Vec<Duration>) -> EvidenceResult<Self> {
        if samples.is_empty() {
            return Err(std::io::Error::other("latency sample must not be empty").into());
        }
        samples.sort_unstable();
        let micros = |index: usize| -> EvidenceResult<u64> {
            u64::try_from(
                samples
                    .get(index)
                    .ok_or("percentile exceeds sample")?
                    .as_micros(),
            )
            .map_err(|_| std::io::Error::other("latency exceeds u64 microseconds").into())
        };
        let last = samples.len() - 1;
        let percentile = |numerator: usize| -> EvidenceResult<usize> {
            Ok(samples
                .len()
                .checked_mul(numerator)
                .ok_or("percentile index overflow")?
                .div_ceil(100)
                .checked_sub(1)
                .ok_or("percentile rank is empty")?)
        };
        Ok(Self {
            count: u64::try_from(samples.len())?,
            minimum_us: micros(0)?,
            p50_us: micros(percentile(50)?)?,
            p95_us: micros(percentile(95)?)?,
            p99_us: micros(percentile(99)?)?,
            maximum_us: micros(last)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_uses_sample_count_and_refuses_unrepresentable_measurements() -> EvidenceResult {
        let summary = LatencySummary::from_durations(
            [4, 1, 3, 2]
                .into_iter()
                .map(Duration::from_micros)
                .collect(),
        )?;
        assert_eq!(
            (
                summary.count,
                summary.minimum_us,
                summary.p50_us,
                summary.p95_us,
                summary.p99_us,
                summary.maximum_us
            ),
            (4, 1, 2, 4, 4, 4)
        );
        let one = LatencySummary::from_durations(vec![Duration::from_micros(7)])?;
        assert_eq!(
            (
                one.minimum_us,
                one.p50_us,
                one.p95_us,
                one.p99_us,
                one.maximum_us
            ),
            (7, 7, 7, 7, 7)
        );
        assert!(LatencySummary::from_durations(vec![]).is_err());
        assert!(LatencySummary::from_durations(vec![Duration::MAX]).is_err());
        Ok(())
    }
}
