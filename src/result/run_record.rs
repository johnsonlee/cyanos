//! Score record for one completed outer run inside the task result.

use crate::result::{ScoreBreakdown, ScoreTransition, TaskResultStatus, VerifierFeedback};

/// Score record for one completed outer run inside the task result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRunRecord {
    run_index: usize,
    selected_sample: usize,
    score: ScoreTransition,
    feedback: VerifierFeedback,
}

impl TaskRunRecord {
    /// Creates a run score record relative to the previous task score.
    #[must_use]
    pub const fn new(
        run_index: usize,
        selected_sample: usize,
        score: ScoreTransition,
        feedback: VerifierFeedback,
    ) -> Self {
        Self {
            run_index,
            selected_sample,
            score,
            feedback,
        }
    }

    /// Returns the outer run index.
    #[must_use]
    pub const fn run_index(&self) -> usize {
        self.run_index
    }

    /// Returns the selected sample id for this run.
    #[must_use]
    pub const fn selected_sample(&self) -> usize {
        self.selected_sample
    }

    /// Returns the task score before this run.
    #[must_use]
    pub const fn previous_score(&self) -> ScoreBreakdown {
        self.score.previous_score()
    }

    /// Returns the selected score for this run.
    #[must_use]
    pub const fn selected_score(&self) -> ScoreBreakdown {
        self.score.selected_score()
    }

    /// Returns the score delta from the previous task score.
    #[must_use]
    pub const fn delta(&self) -> i64 {
        self.score.delta()
    }

    /// Returns this run's progress status.
    #[must_use]
    pub const fn status(&self) -> TaskResultStatus {
        self.score.status()
    }

    /// Returns verifier feedback for the selected sample.
    #[must_use]
    pub const fn feedback(&self) -> &VerifierFeedback {
        &self.feedback
    }

    pub(crate) fn render_json(&self, indent: &str) -> String {
        let score_indent = format!("{indent}  ");
        let feedback_indent = format!("{indent}  ");

        format!(
            "{{\n{indent}  \"run_index\": {},\n{indent}  \"selected_sample\": {},\n{indent}  \"previous_score\": {},\n{indent}  \"selected_score\": {},\n{indent}  \"delta\": {},\n{indent}  \"status\": {},\n{indent}  \"feedback\": {}\n{indent}}}",
            self.run_index(),
            self.selected_sample(),
            self.previous_score().render_json(&score_indent),
            self.selected_score().render_json(&score_indent),
            self.delta(),
            crate::json::string(self.status().as_str()),
            self.feedback().to_json(&feedback_indent)
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        QualityTier, SampleScore, ScoreBreakdown, ScoreTransition, TaskResultStatus, TaskRunRecord,
        VerifierFeedback,
    };

    fn score(total: i64, tier: QualityTier) -> ScoreBreakdown {
        ScoreBreakdown::new(
            SampleScore::new(total),
            tier,
            SampleScore::new(1),
            SampleScore::new(2),
            SampleScore::new(3),
        )
    }

    #[test]
    fn exposes_run_record_fields() {
        let transition = ScoreTransition::new(
            score(20, QualityTier::TestFailed),
            score(25, QualityTier::PartialSuccess),
            score(100, QualityTier::Passed),
        );
        let feedback = VerifierFeedback::new(
            "runs/1/samples/1/eval.json".to_owned(),
            "tests passed".to_owned(),
            "none".to_owned(),
        );
        let record = TaskRunRecord::new(4, 3, transition, feedback);

        assert_eq!(record.run_index(), 4);
        assert_eq!(record.selected_sample(), 3);
        assert_eq!(record.previous_score().total().as_i64(), 20);
        assert_eq!(record.selected_score().total().as_i64(), 25);
        assert_eq!(
            record.selected_score().quality_tier(),
            QualityTier::PartialSuccess
        );
        assert_eq!(record.selected_score().benchmark().as_i64(), 2);
        assert_eq!(record.selected_score().judge().as_i64(), 3);
        assert_eq!(record.delta(), 5);
        assert_eq!(record.status(), TaskResultStatus::Improved);
    }
}
