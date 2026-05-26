//! Structured score components stored in the task result.

use crate::json;
use crate::{r#loop::SampleScore, score::QualityTier};

/// Structured score components stored in the task result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScoreBreakdown {
    total: SampleScore,
    quality_tier: QualityTier,
    tests: SampleScore,
    benchmark: SampleScore,
    judge: SampleScore,
}

impl ScoreBreakdown {
    /// Creates a score breakdown.
    #[must_use]
    pub const fn new(
        total: SampleScore,
        quality_tier: QualityTier,
        tests: SampleScore,
        benchmark: SampleScore,
        judge: SampleScore,
    ) -> Self {
        Self {
            total,
            quality_tier,
            tests,
            benchmark,
            judge,
        }
    }

    /// Returns the total composite score.
    #[must_use]
    pub const fn total(&self) -> SampleScore {
        self.total
    }

    /// Returns the quality tier component.
    #[must_use]
    pub const fn quality_tier(&self) -> QualityTier {
        self.quality_tier
    }

    /// Returns the functional test score component.
    #[must_use]
    pub const fn tests(&self) -> SampleScore {
        self.tests
    }

    /// Returns the benchmark score component.
    #[must_use]
    pub const fn benchmark(&self) -> SampleScore {
        self.benchmark
    }

    /// Returns the LLM judge score component.
    #[must_use]
    pub const fn judge(&self) -> SampleScore {
        self.judge
    }

    pub(crate) fn render_json(&self, indent: &str) -> String {
        format!(
            "{{\n{indent}  \"total\": {},\n{indent}  \"quality_tier\": {},\n{indent}  \"tests\": {},\n{indent}  \"benchmark\": {},\n{indent}  \"judge\": {}\n{indent}}}",
            self.total().as_i64(),
            json::string(self.quality_tier().as_str()),
            self.tests().as_i64(),
            self.benchmark().as_i64(),
            self.judge().as_i64()
        )
    }
}
