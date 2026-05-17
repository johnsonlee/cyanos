//! Composite scoring for SSD sample selection.

use crate::SampleScore;

const SCORE_SCALE: i64 = 10_000;
const MAX_JUDGE_SCORE: u8 = 100;
const JUDGE_SCORE_TO_BASIS_POINTS: i64 = 100;

/// Coarse functional outcome for a sample.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum QualityTier {
    /// Code did not compile.
    CompileFailed,
    /// Code compiled but tests failed.
    TestFailed,
    /// Code partially satisfies the task.
    PartialSuccess,
    /// Code satisfies functional requirements.
    Passed,
}

impl QualityTier {
    /// Returns the runtime artifact label for this tier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CompileFailed => "compile_failed",
            Self::TestFailed => "test_failed",
            Self::PartialSuccess => "partial_success",
            Self::Passed => "passed",
        }
    }

    const fn allows_structural_score(self) -> bool {
        !matches!(self, Self::CompileFailed)
    }
}

/// Structural gate result component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TestScore {
    passed: u32,
    failed: u32,
}

impl TestScore {
    /// Creates a test score.
    #[must_use]
    pub const fn new(passed: u32, failed: u32) -> Self {
        Self { passed, failed }
    }

    fn structural_basis_points(self, quality_tier: QualityTier) -> i64 {
        if !quality_tier.allows_structural_score() {
            return 0;
        }

        let passed = i64::from(self.passed);
        let total = passed + i64::from(self.failed);
        if total == 0 {
            return if quality_tier == QualityTier::Passed {
                SCORE_SCALE
            } else {
                0
            };
        }

        passed * SCORE_SCALE / total
    }
}

/// Normalized benchmark recall component in basis points.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BenchmarkScore {
    value_basis_points: i64,
}

impl BenchmarkScore {
    /// Creates a benchmark recall score in basis points.
    #[must_use]
    pub const fn new(value_basis_points: i64) -> Self {
        Self { value_basis_points }
    }

    fn normalized_basis_points(self) -> i64 {
        self.value_basis_points.clamp(0, SCORE_SCALE)
    }
}

/// Normalized LLM judge recall component from 0 to 100.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LlmJudgeScore {
    value: u8,
}

impl LlmJudgeScore {
    /// Creates an LLM judge score.
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self { value }
    }

    fn normalized_basis_points(self) -> i64 {
        i64::from(self.value.min(MAX_JUDGE_SCORE)) * JUDGE_SCORE_TO_BASIS_POINTS
    }
}

/// Composite sample scoring input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompositeScoreInput {
    quality_tier: QualityTier,
    test_score: TestScore,
    benchmark_score: BenchmarkScore,
    llm_judge_score: LlmJudgeScore,
}

impl CompositeScoreInput {
    /// Creates a composite score input.
    #[must_use]
    pub const fn new(
        quality_tier: QualityTier,
        test_score: TestScore,
        benchmark_score: BenchmarkScore,
        llm_judge_score: LlmJudgeScore,
    ) -> Self {
        Self {
            quality_tier,
            test_score,
            benchmark_score,
            llm_judge_score,
        }
    }
}

/// Multiplicative structural-recall score model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompositeScoreModel {
    scale: i64,
}

impl Default for CompositeScoreModel {
    fn default() -> Self {
        Self::new()
    }
}

impl CompositeScoreModel {
    /// Creates a composite score model.
    #[must_use]
    pub const fn new() -> Self {
        Self { scale: SCORE_SCALE }
    }

    /// Scores a sample as `structural * recall`.
    #[must_use]
    pub fn score(&self, input: CompositeScoreInput) -> SampleScore {
        let structural = input.test_score.structural_basis_points(input.quality_tier);
        let recall = recall_product([
            input.benchmark_score.normalized_basis_points(),
            input.llm_judge_score.normalized_basis_points(),
        ]);
        let value = structural * recall / self.scale;

        SampleScore::new(value)
    }
}

fn recall_product<const N: usize>(items: [i64; N]) -> i64 {
    items
        .into_iter()
        .fold(SCORE_SCALE, |score, item| score * item / SCORE_SCALE)
}

#[cfg(test)]
mod tests {
    use crate::{
        BenchmarkScore, CompositeScoreInput, CompositeScoreModel, LlmJudgeScore, QualityTier,
        TestScore,
    };

    #[test]
    fn compile_failure_zeroes_structural_score() {
        let model = CompositeScoreModel::default();
        let compile_failed = model.score(CompositeScoreInput::new(
            QualityTier::CompileFailed,
            TestScore::new(10, 0),
            BenchmarkScore::new(10_000),
            LlmJudgeScore::new(100),
        ));
        let test_failed = model.score(CompositeScoreInput::new(
            QualityTier::TestFailed,
            TestScore::new(8, 2),
            BenchmarkScore::new(10_000),
            LlmJudgeScore::new(100),
        ));

        assert_eq!(compile_failed.as_i64(), 0);
        assert_eq!(test_failed.as_i64(), 8_000);
    }

    #[test]
    fn scores_samples_as_structural_times_recall_product() {
        let model = CompositeScoreModel::new();
        let score = model.score(CompositeScoreInput::new(
            QualityTier::Passed,
            TestScore::new(10, 0),
            BenchmarkScore::new(8_000),
            LlmJudgeScore::new(90),
        ));

        assert_eq!(score.as_i64(), 7_200);
    }

    #[test]
    fn clamps_recall_items_to_normalized_range() {
        let model = CompositeScoreModel::new();
        let score = model.score(CompositeScoreInput::new(
            QualityTier::Passed,
            TestScore::new(0, 0),
            BenchmarkScore::new(20_000),
            LlmJudgeScore::new(200),
        ));

        assert_eq!(score.as_i64(), 10_000);
    }

    #[test]
    fn partial_success_without_tests_zeroes_structural_score() {
        let model = CompositeScoreModel::new();
        let score = model.score(CompositeScoreInput::new(
            QualityTier::PartialSuccess,
            TestScore::new(0, 0),
            BenchmarkScore::new(10_000),
            LlmJudgeScore::new(100),
        ));

        assert_eq!(score.as_i64(), 0);
    }
}
