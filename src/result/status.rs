//! Task-level progress status for evolution results.

use crate::result::ScoreBreakdown;

/// Coarse progress state for the task result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskResultStatus {
    /// Current score is equal to the task baseline.
    Baseline,
    /// Current score improved from the comparison score.
    Improved,
    /// Current score regressed from the comparison score.
    Regressed,
    /// Current score reached the configured perfect threshold.
    Perfect,
}

impl TaskResultStatus {
    /// Returns the runtime artifact label for this status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Improved => "improved",
            Self::Regressed => "regressed",
            Self::Perfect => "perfect",
        }
    }

    pub(crate) const fn classify(
        reference: ScoreBreakdown,
        score: ScoreBreakdown,
        perfect: ScoreBreakdown,
    ) -> Self {
        if score.total().as_i64() >= perfect.total().as_i64() {
            Self::Perfect
        } else if score.total().as_i64() > reference.total().as_i64() {
            Self::Improved
        } else if score.total().as_i64() < reference.total().as_i64() {
            Self::Regressed
        } else {
            Self::Baseline
        }
    }
}
