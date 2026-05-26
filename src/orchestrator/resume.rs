//! Run orchestration state recovered from durable runtime evidence.

use std::{fs, io, path::PathBuf};

use crate::{AppError, RuntimeCheckpoint, TaskLayout, TaskResultSnapshot};

const DELIVERY_PR_FILE: &str = "delivery-pr.txt";
const DELIVERY_PR_HEAD_PREFIX: &str = "head=";
const DELIVERY_PR_URL_PREFIX: &str = "url=";
const READ_DELIVERY_PR_ACTION: &str = "read delivery PR identity";
const READ_TASK_RESULT_ACTION: &str = "read task result";
const WRITE_DELIVERY_PR_ACTION: &str = "write delivery PR identity";
const PARSE_TASK_RESULT_ACTION: &str = "parse task result";
const RESUME_STATE_PROMPT_EVOLUTION: &str = "prompt_evolution";
const RESUME_STATE_SAMPLE_EXECUTION: &str = "sample_execution";
#[cfg(test)]
const STATE_BRIEF_FROZEN: &str = "brief_frozen";
const STATE_EVALUATED_SAMPLES: &str = "evaluated_samples";
const STATE_GLOBAL_BEST_ACCEPTED: &str = "global_best_accepted";
const STATE_GLOBAL_BEST_REJECTED: &str = "global_best_rejected";
const STATE_INTAKE: &str = "intake";
const STATE_NEW: &str = "new";
const STATE_PR_FEEDBACK: &str = "pr_feedback";
const STATE_PR_OPENED: &str = "pr_opened";
const STATE_PR_READY: &str = "pr_ready";
const STATE_PROMOTION: &str = "promotion";
const STATE_PROMPT_EVOLVED: &str = "prompt_evolved";
const STATE_RUNNING_SAMPLES: &str = "running_samples";
const STATE_TERMINAL_FAILURE: &str = "terminal_failure";
const TSV_HEAD_INDEX: usize = 1;
const TSV_TITLE_INDEX: usize = 2;
const TSV_URL_INDEX: usize = 0;
const TSV_BODY_INDEX: usize = 3;

/// Persisted identity of the task delivery pull request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryPrIdentity {
    /// Pull request URL.
    pub url: String,
    /// Pull request head branch.
    pub head: String,
}

impl DeliveryPrIdentity {
    /// Creates a delivery pull request identity.
    #[must_use]
    pub fn new(url: impl Into<String>, head: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            head: head.into(),
        }
    }
}

/// Durable state from which a rerun should resume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResumeState {
    /// No durable runtime evidence exists yet.
    New,
    /// Resume task intake and brief freezing.
    Intake,
    /// Resume or classify sample execution.
    SampleExecution,
    /// Resume classification of a persisted selected sample.
    SampleEvaluation,
    /// Resume prompt evolution.
    PromptEvolution,
    /// Resume promotion of the accepted best sample.
    Promotion,
    /// Resume after PR creation/update.
    PrOpened,
    /// Resume from PR readiness or review feedback.
    PrFeedback,
    /// Resume from a persisted terminal failure.
    TerminalFailure,
    /// Resume from a PR-ready checkpoint.
    PrReady,
}

impl ResumeState {
    /// Returns the durable state label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => STATE_NEW,
            Self::Intake => STATE_INTAKE,
            Self::SampleExecution => RESUME_STATE_SAMPLE_EXECUTION,
            Self::SampleEvaluation => STATE_EVALUATED_SAMPLES,
            Self::PromptEvolution => RESUME_STATE_PROMPT_EVOLUTION,
            Self::Promotion => STATE_PROMOTION,
            Self::PrOpened => STATE_PR_OPENED,
            Self::PrFeedback => STATE_PR_FEEDBACK,
            Self::TerminalFailure => STATE_TERMINAL_FAILURE,
            Self::PrReady => STATE_PR_READY,
        }
    }
}

/// Resume decision derived from runtime ledger, result snapshot, and delivery PR evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumePlan {
    /// Durable state to continue from.
    pub state: ResumeState,
    /// Highest completed outer run index recorded in `result.json`.
    pub last_run_index: usize,
    /// Next outer run index Cyanos should schedule.
    pub next_run_index: usize,
    /// Persisted delivery PR identity, when known.
    pub delivery_pr: Option<DeliveryPrIdentity>,
}

impl ResumePlan {
    /// Returns whether this plan resumes existing runtime state.
    #[must_use]
    pub const fn is_resume(&self) -> bool {
        !matches!(self.state, ResumeState::New)
    }
}

/// Plans task resume from durable runtime evidence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResumePlanner;

impl ResumePlanner {
    /// Builds a resume plan for one task.
    ///
    /// # Errors
    ///
    /// Returns an error when durable runtime files exist but cannot be read or parsed.
    pub fn plan(task: &TaskLayout) -> Result<ResumePlan, AppError> {
        let last_state = RuntimeCheckpoint::last_ledger_state(task)?;
        let state = last_state
            .as_deref()
            .map_or(ResumeState::New, resume_state_from_ledger_state);
        let last_run_index = last_result_run_index(task)?;
        let next_run_index = next_run_index_for_resume(state, last_run_index);
        let delivery_pr = read_delivery_pr_identity(task)?;
        Ok(ResumePlan {
            state,
            last_run_index,
            next_run_index,
            delivery_pr,
        })
    }
}

/// Returns the delivery PR identity file path for a task.
#[must_use]
pub fn delivery_pr_identity_path(task: &TaskLayout) -> PathBuf {
    task.root().join(DELIVERY_PR_FILE)
}

/// Loads the persisted delivery PR identity for a task.
///
/// # Errors
///
/// Returns an error when an existing identity file cannot be read.
pub fn read_delivery_pr_identity(
    task: &TaskLayout,
) -> Result<Option<DeliveryPrIdentity>, AppError> {
    let path = delivery_pr_identity_path(task);
    match fs::read_to_string(&path) {
        Ok(content) => Ok(delivery_pr_identity_from_text(
            &content,
            task.pr_branch_name().as_str(),
        )),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(AppError::io(READ_DELIVERY_PR_ACTION, source)),
    }
}

/// Persists the delivery PR identity for a task.
///
/// # Errors
///
/// Returns an error when the identity file cannot be written.
pub fn write_delivery_pr_identity(
    task: &TaskLayout,
    identity: &DeliveryPrIdentity,
) -> Result<(), AppError> {
    let path = delivery_pr_identity_path(task);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| AppError::io(WRITE_DELIVERY_PR_ACTION, source))?;
    }
    fs::write(
        path,
        format!(
            "{DELIVERY_PR_URL_PREFIX}{}\n{DELIVERY_PR_HEAD_PREFIX}{}\n",
            identity.url, identity.head
        ),
    )
    .map_err(|source| AppError::io(WRITE_DELIVERY_PR_ACTION, source))
}

/// Parses a delivery PR identity from persisted runtime text.
#[must_use]
pub fn delivery_pr_identity_from_text(
    content: &str,
    fallback_head: &str,
) -> Option<DeliveryPrIdentity> {
    let mut url = None;
    let mut head = None;
    for line in content.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix(DELIVERY_PR_URL_PREFIX) {
            url = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix(DELIVERY_PR_HEAD_PREFIX) {
            head = Some(value.trim().to_owned());
        } else if url.is_none() {
            url = Some(line.to_owned());
        }
    }
    let url = url.filter(|value| !value.is_empty())?;
    Some(DeliveryPrIdentity::new(
        url,
        head.filter(|value| !value.is_empty())
            .unwrap_or_else(|| fallback_head.to_owned()),
    ))
}

/// Parses unique delivery PR candidates from `gh pr list` TSV output.
#[must_use]
pub fn delivery_pr_candidates_from_tsv(
    output: &[u8],
    fallback_head: &str,
    task_id: &str,
) -> Vec<DeliveryPrIdentity> {
    let text = String::from_utf8_lossy(output);
    let mut identities = Vec::new();
    for line in text.lines() {
        let fields = line.split('\t').collect::<Vec<_>>();
        let Some(url) = fields.get(TSV_URL_INDEX).map(|value| value.trim()) else {
            continue;
        };
        if url.is_empty() {
            continue;
        }
        let title = fields.get(TSV_TITLE_INDEX).map_or("", |value| value.trim());
        let body = fields.get(TSV_BODY_INDEX).map_or("", |value| value.trim());
        if !delivery_pr_owns_task(title, body, task_id) {
            continue;
        }
        let head = fields
            .get(TSV_HEAD_INDEX)
            .map_or(fallback_head, |value| value.trim());
        let head = if head.is_empty() { fallback_head } else { head };
        let identity = DeliveryPrIdentity::new(url, head);
        if !identities.iter().any(|existing| existing == &identity) {
            identities.push(identity);
        }
    }
    identities
}

/// Returns the stable machine-readable PR task marker.
#[must_use]
pub fn delivery_pr_marker(task_id: &str) -> String {
    format!("<!--cyanos:task={task_id}-->")
}

fn delivery_pr_owns_task(title: &str, body: &str, task_id: &str) -> bool {
    body.contains(&delivery_pr_marker(task_id))
        || exact_issue_reference(title, task_id)
        || exact_issue_reference(body, task_id)
}

fn exact_issue_reference(text: &str, task_id: &str) -> bool {
    let needle = format!("#{task_id}");
    for (start, _) in text.match_indices(&needle) {
        let end = start.saturating_add(needle.len());
        let previous_ok = text
            .get(..start)
            .and_then(|prefix| prefix.chars().next_back())
            .is_none_or(|character| !character.is_ascii_alphanumeric());
        let next_ok = text
            .get(end..)
            .and_then(|suffix| suffix.chars().next())
            .is_none_or(|character| !character.is_ascii_alphanumeric());
        if previous_ok && next_ok {
            return true;
        }
    }
    false
}

/// Maps a ledger state label to a resume state.
#[must_use]
pub fn resume_state_from_ledger_state(state: &str) -> ResumeState {
    match state {
        STATE_RUNNING_SAMPLES => ResumeState::SampleExecution,
        STATE_EVALUATED_SAMPLES | STATE_GLOBAL_BEST_REJECTED => ResumeState::SampleEvaluation,
        STATE_PROMPT_EVOLVED => ResumeState::PromptEvolution,
        STATE_GLOBAL_BEST_ACCEPTED => ResumeState::Promotion,
        STATE_PR_OPENED => ResumeState::PrOpened,
        STATE_PR_FEEDBACK => ResumeState::PrFeedback,
        STATE_TERMINAL_FAILURE => ResumeState::TerminalFailure,
        STATE_PR_READY => ResumeState::PrReady,
        _ => ResumeState::Intake,
    }
}

/// Returns the next outer run index for a resume state.
#[must_use]
pub const fn next_run_index_for_resume(state: ResumeState, last_run_index: usize) -> usize {
    match state {
        ResumeState::New | ResumeState::Intake => 1,
        ResumeState::SampleExecution if last_run_index == 0 => 1,
        ResumeState::SampleExecution
        | ResumeState::PromptEvolution
        | ResumeState::PrOpened
        | ResumeState::PrFeedback
        | ResumeState::TerminalFailure => last_run_index.saturating_add(1),
        ResumeState::SampleEvaluation | ResumeState::Promotion | ResumeState::PrReady => {
            if last_run_index > 0 {
                last_run_index
            } else {
                1
            }
        }
    }
}

fn last_result_run_index(task: &TaskLayout) -> Result<usize, AppError> {
    match fs::read_to_string(task.result()) {
        Ok(content) => {
            let snapshot = TaskResultSnapshot::parse(&content).map_err(|source| {
                AppError::io(
                    PARSE_TASK_RESULT_ACTION,
                    io::Error::new(io::ErrorKind::InvalidData, source),
                )
            })?;
            Ok(snapshot.max_run_index().unwrap_or(0))
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(AppError::io(READ_TASK_RESULT_ACTION, source)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{
        CyanosHome, DeliveryPrIdentity, ProjectId, ResumePlanner, ResumeState, RuntimeCheckpoint,
        TaskId, delivery_pr_candidates_from_tsv, delivery_pr_identity_from_text,
        next_run_index_for_resume, resume_state_from_ledger_state,
    };

    #[test]
    fn plans_resume_from_runtime_evidence() -> Result<(), Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("cyanos-orchestrator-resume-{}", std::process::id()));
        let task = CyanosHome::new(root.join("home"))
            .project(ProjectId::new("owner/repo")?)
            .task(TaskId::new("123".to_owned())?);
        fs::create_dir_all(task.root())?;
        RuntimeCheckpoint::append_ledger(&task, super::STATE_BRIEF_FROZEN, "frozen")?;
        RuntimeCheckpoint::append_ledger(&task, super::STATE_PR_FEEDBACK, "needs changes")?;
        fs::write(
            task.result(),
            "{\n  \"status\": \"improved\",\n  \"runs\": [\n    {\n      \"run_index\": 1,\n      \"selected_sample\": 1,\n      \"status\": \"improved\",\n      \"feedback\": {\n        \"eval_path\": \"runs/1/samples/1/eval.json\",\n        \"next_action\": \"continue\"\n      }\n    },\n    {\n      \"run_index\": 2,\n      \"selected_sample\": 1,\n      \"status\": \"improved\",\n      \"feedback\": {\n        \"eval_path\": \"runs/2/samples/1/eval.json\",\n        \"next_action\": \"continue\"\n      }\n    }\n  ]\n}\n",
        )?;
        crate::write_delivery_pr_identity(
            &task,
            &DeliveryPrIdentity::new(
                "https://github.com/owner/repo/pull/456",
                "feature/installable-alpha",
            ),
        )?;

        let plan = ResumePlanner::plan(&task)?;

        assert_eq!(plan.state, ResumeState::PrFeedback);
        assert_eq!(plan.last_run_index, 2);
        assert_eq!(plan.next_run_index, 3);
        assert_eq!(
            plan.delivery_pr
                .ok_or_else(|| std::io::Error::other("missing delivery PR"))?
                .head,
            "feature/installable-alpha"
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn parses_delivery_pr_identity_and_resume_edges() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(ResumeState::New.as_str(), "new");
        assert_eq!(ResumeState::Intake.as_str(), "intake");
        assert_eq!(ResumeState::SampleExecution.as_str(), "sample_execution");
        assert_eq!(ResumeState::SampleEvaluation.as_str(), "evaluated_samples");
        assert_eq!(ResumeState::PromptEvolution.as_str(), "prompt_evolution");
        assert_eq!(ResumeState::PrOpened.as_str(), "pr_opened");
        assert_eq!(ResumeState::TerminalFailure.as_str(), "terminal_failure");
        assert_eq!(
            next_run_index_for_resume(ResumeState::SampleExecution, 0),
            1
        );
        assert_eq!(
            next_run_index_for_resume(ResumeState::PromptEvolution, 2),
            3
        );
        assert_eq!(
            next_run_index_for_resume(ResumeState::SampleEvaluation, 2),
            2
        );
        assert_eq!(next_run_index_for_resume(ResumeState::Promotion, 2), 2);
        assert_eq!(next_run_index_for_resume(ResumeState::PrReady, 0), 1);
        assert_eq!(
            resume_state_from_ledger_state(super::STATE_GLOBAL_BEST_ACCEPTED),
            ResumeState::Promotion
        );
        assert_eq!(
            resume_state_from_ledger_state(super::STATE_RUNNING_SAMPLES),
            ResumeState::SampleExecution
        );
        assert_eq!(
            resume_state_from_ledger_state(super::STATE_EVALUATED_SAMPLES),
            ResumeState::SampleEvaluation
        );
        assert_eq!(
            resume_state_from_ledger_state(super::STATE_GLOBAL_BEST_REJECTED),
            ResumeState::SampleEvaluation
        );
        assert_eq!(
            resume_state_from_ledger_state(super::STATE_PROMPT_EVOLVED),
            ResumeState::PromptEvolution
        );
        assert_eq!(
            resume_state_from_ledger_state(super::STATE_PR_OPENED),
            ResumeState::PrOpened
        );
        assert_eq!(
            resume_state_from_ledger_state(super::STATE_TERMINAL_FAILURE),
            ResumeState::TerminalFailure
        );
        assert_eq!(
            resume_state_from_ledger_state(super::STATE_PR_READY),
            ResumeState::PrReady
        );
        assert_eq!(
            resume_state_from_ledger_state("unknown"),
            ResumeState::Intake
        );
        assert_eq!(
            delivery_pr_identity_from_text("https://github.com/owner/repo/pull/1\n", "feature/123")
                .ok_or_else(|| std::io::Error::other("missing legacy delivery PR"))?
                .head,
            "feature/123"
        );
        let candidates = delivery_pr_candidates_from_tsv(
            b"https://github.com/owner/repo/pull/1\tfeature/a\tCyanos task 123\t<!--cyanos:task=123-->\nhttps://github.com/owner/repo/pull/1\tfeature/a\tCyanos task 123\t<!--cyanos:task=123-->\nhttps://github.com/owner/repo/pull/2\tfeature/b\tWrong task\tmentions #1234 and #1235\nhttps://github.com/owner/repo/pull/3\tfeature/c\tFix #123\tlegacy exact issue reference\n",
            "feature/123",
            "123",
        );
        assert_eq!(candidates.len(), 2);
        Ok(())
    }

    #[test]
    fn plans_new_and_rejects_malformed_result_state() -> Result<(), Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("cyanos-orchestrator-new-{}", std::process::id()));
        let task = CyanosHome::new(root.join("home"))
            .project(ProjectId::new("owner/repo")?)
            .task(TaskId::new("123".to_owned())?);
        fs::create_dir_all(task.root())?;

        let plan = ResumePlanner::plan(&task)?;
        assert!(!plan.is_resume());
        assert_eq!(plan.state, ResumeState::New);
        assert_eq!(plan.next_run_index, 1);
        assert!(crate::read_delivery_pr_identity(&task)?.is_none());
        assert!(crate::delivery_pr_identity_from_text("url=\nhead=\n", "feature/123").is_none());
        assert!(crate::delivery_pr_candidates_from_tsv(b"\n\t\n", "feature/123", "123").is_empty());

        RuntimeCheckpoint::append_ledger(&task, super::STATE_PR_OPENED, "opened")?;
        fs::write(task.result(), "{}")?;
        let result = ResumePlanner::plan(&task);
        assert!(matches!(result, Err(crate::AppError::Io { .. })));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture paths must be created")]
    #[expect(
        clippy::assertions_on_result_states,
        reason = "test asserts unreadable runtime evidence errors"
    )]
    fn reports_result_and_delivery_identity_read_errors() {
        let root = std::env::temp_dir().join(format!(
            "cyanos-orchestrator-read-errors-{}",
            std::process::id()
        ));
        let task = CyanosHome::new(root.join("home"))
            .project(ProjectId::new("owner/repo").expect("valid project id"))
            .task(TaskId::new("123".to_owned()).expect("valid task id"));
        fs::create_dir_all(task.root()).expect("create task root");

        fs::create_dir_all(crate::delivery_pr_identity_path(&task))
            .expect("create unreadable delivery identity");
        assert!(crate::read_delivery_pr_identity(&task).is_err());
        fs::remove_dir_all(crate::delivery_pr_identity_path(&task))
            .expect("remove unreadable delivery identity");

        RuntimeCheckpoint::append_ledger(&task, super::STATE_PR_OPENED, "opened")
            .expect("append ledger");
        fs::create_dir_all(task.result()).expect("create unreadable result path");
        assert!(ResumePlanner::plan(&task).is_err());

        fs::remove_dir_all(root).expect("remove orchestrator root");
    }
}
