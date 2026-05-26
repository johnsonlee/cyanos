//! Verification, scoring, and judge-feedback boundary.

mod sample;

pub use sample::{
    JUDGE_PERCENT_TO_BASIS_POINTS, JUDGE_RUBRIC_RULE, JUDGE_SYSTEM_PROMPT, JudgeFinding,
    JudgeIdentity, JudgeReview, SampleVerification, VerifierCommand,
    evaluate_sample_worktree_with_verifiers, format_coverage_evidence, judge_recall_from_findings,
    judge_review_prompt, parse_coverage_basis_points, parse_judge_review,
};
