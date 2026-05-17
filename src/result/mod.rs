//! Task-level result state for evolution progress.

mod run_record;
mod score_breakdown;
mod score_transition;
mod status;
mod task_result;
mod verifier_feedback;

pub use run_record::TaskRunRecord;
pub use score_breakdown::ScoreBreakdown;
pub use score_transition::ScoreTransition;
pub use status::TaskResultStatus;
pub use task_result::{TaskResult, TaskResultParseError, TaskResultSnapshot};
pub use verifier_feedback::VerifierFeedback;
