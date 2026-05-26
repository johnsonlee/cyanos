//! Core library for Cyanos, a model-agnostic coding agent CLI.

pub mod agent;
pub mod cli;
pub mod dependency;
pub mod error;
pub mod eval;
pub mod github;
mod json;
pub mod r#loop;
pub mod orchestrator;
pub mod prompt;
pub mod result;
pub mod runtime;
pub mod score;
pub mod task;
pub mod tui;
pub mod verifier;

pub(crate) mod terms;

pub use runtime::{
    best as runtime_best, checkpoint as runtime_checkpoint, process, project, workspace,
};

pub use agent::{
    AdapterRegistry, Agent, AgentCommandRunner, AgentExecutionMode, AgentOutput, AgentRequest,
    AgentRuntime, AgentRuntimeError, AgentStreamEvent, AgentStreamObserver, AgentStreamParser,
    AgentStreamToolCallEvent, AgentTurn, CommandSpec, ModelSelection, NoopAgentStreamObserver,
    SystemAgentCommandRunner, SystemAgentRuntime,
};
pub use cli::{CliCommand, CliError, CliParser, CyanosCli, InitCommand, Invocation, RunCommand};
pub use dependency::{
    CommandProbe, DependencyCheck, DependencyChecker, DependencyError, DependencyPlan,
    DependencyReport, DependencyRequirement, SystemCommandProbe,
};
pub use error::{AppError, ErrorReport, ErrorReporter, ExitStatus};
pub use eval::{
    Evaluation, EvaluationFinding, EvaluationParseError, EvaluationSnapshot, EvaluationVerdict,
    Evaluator, NonEmptyOutputEvaluator,
};
pub use github::parse_unresolved_review_thread_count;
pub use github::{
    ReviewCommentRecord, ReviewThreadEvidence, fixed_review_thread_ids_from_verifier_evidence,
    review_comment_records_from_tsv, review_comment_to_finding,
    review_feedback_from_comment_records, review_feedback_from_comment_records_with_evidence,
    review_thread_has_managed_fixed_resolution, unresolved_fixed_thread_ids,
    unresolved_fixed_thread_ids_with_evidence, unresolved_outdated_thread_ids,
};
pub use r#loop::{
    AgentRuntimeFactory, HighestScoreSelector, Hypothesis, HypothesisPromptReviser, InnerPrompt,
    LineageStrategy, LoopAttempt, LoopConfig, LoopError, LoopReport, LoopStatus, LoopTask,
    NestedLoop, NestedLoopReport, NoopSampleObserverFactory, OuterPrompt, PromptReviser,
    PromptRevision, SampleId, SampleObserverFactory, SampleOrigin, SamplePlan, SamplePlanner,
    SampleResult, SampleScore, SampleScorer, SampleSelectDistill, SsdConfig, SsdConfigError,
    SsdReport, SsdRunPlan, TaskKind, TaskShape, TwoLayerLoop, UserIntent, VerdictSampleScorer,
};
pub use orchestrator::{
    DeliveryPrIdentity, PrContinuation, ResumePlan, ResumePlanner, ResumeState, RunContext,
    RunLifecycleHooks, RunOrchestrator, RunSampleBatch, RunTaskBrief, SampleRunOutcome,
    delivery_pr_candidates_from_tsv, delivery_pr_identity_from_text, delivery_pr_identity_path,
    delivery_pr_marker, new_task_result, next_run_index_for_resume, read_delivery_pr_identity,
    resume_state_from_ledger_state, select_sample, write_delivery_pr_identity,
};
pub use project::{
    GitRunner, HomeResolver, ProjectConfig, ProjectConfigParseError, ProjectInitError,
    ProjectInitReport, ProjectInitRequest, ProjectInitializer, SystemGitRunner,
};
pub use prompt::{
    EMBEDDED_INNER_PROMPT, EMBEDDED_OUTER_PROMPT, EmbeddedPromptLoader, PromptLoadError,
    PromptLoader, PromptPaths, PromptSet, PromptSnapshot, PromptWriteError, RuntimePromptReport,
    RuntimePromptWriter,
};
pub use result::{
    ScoreBreakdown, ScoreTransition, TaskResult, TaskResultParseError, TaskResultSnapshot,
    TaskResultStatus, TaskRunRecord, VerifierFeedback,
};
pub use runtime_best::{BestRecord, BestRecordParseError, read_best_record, write_best_record};
pub use runtime_checkpoint::RuntimeCheckpoint;
pub use score::{
    BenchmarkScore, CompositeScoreInput, CompositeScoreModel, LlmJudgeScore, QualityTier, TestScore,
};
pub use task::{
    SystemTaskGitRunner, TaskGitRunner, TaskRunError, TaskRunReport, TaskRunRequest, TaskRunner,
};
pub use tui::{SampleTui, SampleTuiObserver};
pub use verifier::{
    JUDGE_PERCENT_TO_BASIS_POINTS, JUDGE_RUBRIC_RULE, JUDGE_SYSTEM_PROMPT, JudgeFinding,
    JudgeIdentity, JudgeReview, SampleVerification, VerifierCommand,
    evaluate_sample_worktree_with_verifiers, format_coverage_evidence, judge_recall_from_findings,
    judge_review_prompt, parse_coverage_basis_points, parse_judge_review,
};
pub use workspace::{
    CyanosHome, IdentifierError, ProjectId, ProjectLayout, ProjectLock, ProjectLockError,
    PromotionStrategy, TaskId, TaskLayout,
};
