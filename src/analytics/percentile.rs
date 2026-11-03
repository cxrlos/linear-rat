use jiff::SignedDuration;

/// The median and 85th percentile of a set of durations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Percentiles {
    pub p50: SignedDuration,
    pub p85: SignedDuration,
    pub samples: usize,
}

impl Percentiles {
    /// Nearest-rank percentiles: the p-th percentile is the sample at rank
    /// `ceil(p / 100 * n)` in ascending order, so every result is a real
    /// sample. No samples gives `None`; one sample is both p50 and p85; with
    /// two, p50 is the lower and p85 the higher.
    pub fn nearest_rank(mut samples: Vec<SignedDuration>) -> Option<Self> {
        samples.sort_unstable();
        Some(Self {
            p50: rank(&samples, 50)?,
            p85: rank(&samples, 85)?,
            samples: samples.len(),
        })
    }
}

fn rank(sorted: &[SignedDuration], percent: usize) -> Option<SignedDuration> {
    let rank = (percent * sorted.len()).div_ceil(100);
    sorted.get(rank.checked_sub(1)?).copied()
}
