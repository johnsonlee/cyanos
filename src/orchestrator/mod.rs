//! Resume, SSD, promotion, and delivery orchestration boundary.

mod resume;
mod run;

pub use resume::{
    DeliveryPrIdentity, ResumePlan, ResumePlanner, ResumeState, delivery_pr_candidates_from_tsv,
    delivery_pr_identity_from_text, delivery_pr_identity_path, delivery_pr_marker,
    next_run_index_for_resume, read_delivery_pr_identity, resume_state_from_ledger_state,
    write_delivery_pr_identity,
};
pub use run::{
    PrContinuation, RunContext, RunLifecycleHooks, RunOrchestrator, RunSampleBatch, RunTaskBrief,
    SampleRunOutcome, new_task_result, select_sample,
};
