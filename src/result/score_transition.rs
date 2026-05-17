//! Score transition for one selected outer run.

use crate::result::{ScoreBreakdown, TaskResultStatus};

/// Score transition for one selected outer run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScoreTransition {
    previous_score: ScoreBreakdown,
    selected_score: ScoreBreakdown,
    delta: i64,
    status: TaskResultStatus,
}

impl ScoreTransition {
    /// Creates a score transition.
    #[must_use]
    pub const fn new(
        previous_score: ScoreBreakdown,
        selected_score: ScoreBreakdown,
        perfect_score: ScoreBreakdown,
    ) -> Self {
        Self {
            previous_score,
            selected_score,
            delta: selected_score
                .total()
                .as_i64()
                .saturating_sub(previous_score.total().as_i64()),
            status: TaskResultStatus::classify(previous_score, selected_score, perfect_score),
        }
    }

    /// Returns the task score before this run.
    #[must_use]
    pub const fn previous_score(&self) -> ScoreBreakdown {
        self.previous_score
    }

    /// Returns the selected score for this run.
    #[must_use]
    pub const fn selected_score(&self) -> ScoreBreakdown {
        self.selected_score
    }

    /// Returns the score delta from the previous task score.
    #[must_use]
    pub const fn delta(&self) -> i64 {
        self.delta
    }

    /// Returns this run's progress status.
    #[must_use]
    pub const fn status(&self) -> TaskResultStatus {
        self.status
    }
}
