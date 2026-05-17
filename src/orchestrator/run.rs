//! Library-owned run lifecycle state machine.

use std::{fmt, fs, path::PathBuf};

use crate::{
    AgentRuntimeError, AppError, DependencyChecker, DependencyPlan, Evaluation, HomeResolver,
    Invocation, JudgeIdentity, ProjectConfig, ProjectLayout, ProjectLock, PromptSet, ResumePlan,
    ResumePlanner, RunCommand, RuntimeCheckpoint, RuntimePromptWriter, SampleScore, SampleTui,
    ScoreBreakdown, SystemCommandProbe, SystemTaskGitRunner, TaskId, TaskLayout, TaskResult,
    TaskRunner,
};

const CHECKPOINT_BEST_PROMOTED: &str = "best-promoted";
const DEBUG_OUTER_RUN_LIMIT: &str = "outer_run_limit";
const DEBUG_RUN_INDEX: &str = "run_index";
const STATE_BRIEF_FROZEN: &str = "brief_frozen";
const STATE_EVALUATED_SAMPLES: &str = "evaluated_samples";
const STATE_GLOBAL_BEST_ACCEPTED: &str = "global_best_accepted";
const STATE_GLOBAL_BEST_REJECTED: &str = "global_best_rejected";
const STATE_PROMPT_EVOLVED: &str = "prompt_evolved";
const STATE_RUNNING_SAMPLES: &str = "running_samples";
const STATE_TERMINAL_FAILURE: &str = "terminal_failure";
const PERFECT_SAMPLE_SCORE: i64 = 10_000;

/// Minimal task brief surface the run lifecycle needs after GitHub intake.
pub trait RunTaskBrief {
    /// Returns the issue or PR body to validate against intake rules.
    fn body(&self) -> &str;

    /// Returns the durable runtime source label for the task brief.
    fn source_label(&self) -> PathBuf;

    /// Renders the task prompt source persisted into the runtime.
    fn render_task_source(&self) -> String;
}

/// Immutable run context passed to lifecycle I/O hooks.
pub struct RunContext<'a> {
    invocation: &'a Invocation,
    project: &'a ProjectLayout,
    config: &'a ProjectConfig,
    command: &'a RunCommand,
    task: &'a TaskLayout,
    prompts: &'a PromptSet,
    task_readme: &'a str,
}

impl fmt::Debug for RunContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunContext")
            .field("project", self.project)
            .field("config", self.config)
            .field("command", self.command)
            .field("task", self.task)
            .finish_non_exhaustive()
    }
}

impl<'a> RunContext<'a> {
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "run context groups the lifecycle identities into one hook parameter"
    )]
    fn new(
        invocation: &'a Invocation,
        project: &'a ProjectLayout,
        config: &'a ProjectConfig,
        command: &'a RunCommand,
        task: &'a TaskLayout,
        prompts: &'a PromptSet,
        task_readme: &'a str,
    ) -> Self {
        Self {
            invocation,
            project,
            config,
            command,
            task,
            prompts,
            task_readme,
        }
    }

    /// Returns the parsed command invocation.
    #[must_use]
    pub const fn invocation(&self) -> &Invocation {
        self.invocation
    }

    /// Returns the managed runtime project.
    #[must_use]
    pub const fn project(&self) -> &ProjectLayout {
        self.project
    }

    /// Returns the managed project configuration.
    #[must_use]
    pub const fn config(&self) -> &ProjectConfig {
        self.config
    }

    /// Returns the run command.
    #[must_use]
    pub const fn command(&self) -> &RunCommand {
        self.command
    }

    /// Returns the task runtime layout.
    #[must_use]
    pub const fn task(&self) -> &TaskLayout {
        self.task
    }

    /// Returns the embedded prompt set.
    #[must_use]
    pub const fn prompts(&self) -> &PromptSet {
        self.prompts
    }

    /// Returns the rendered task source prompt.
    #[must_use]
    pub const fn task_readme(&self) -> &str {
        self.task_readme
    }
}

/// One sample execution result with all evidence paths needed downstream.
#[derive(Debug)]
pub struct SampleRunOutcome {
    /// Outer run index that produced this sample.
    pub run_index: usize,
    /// Isolated sample id inside the outer run.
    pub sample_id: usize,
    /// Verifier verdict and findings.
    pub evaluation: Evaluation,
    /// Structured score for sample ordering.
    pub score: ScoreBreakdown,
    /// Persisted sample eval path.
    pub eval_path: PathBuf,
    /// Persisted agent summary path.
    pub summary_path: PathBuf,
    /// Persisted patch path.
    pub patch_path: PathBuf,
}

/// Decision after opening or updating the delivery PR.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrContinuation {
    /// PR has no blocking feedback.
    Ready,
    /// Feedback was found and the next outer run should continue.
    Continue,
}

/// Sample batch context for one SSD outer run.
pub struct RunSampleBatch<'a> {
    /// Run command.
    pub command: &'a RunCommand,
    /// Task runtime layout.
    pub task: &'a TaskLayout,
    /// Outer run index.
    pub run_index: usize,
    /// Current inner prompt.
    pub inner_prompt: &'a str,
    /// Rendered task source prompt.
    pub task_readme: &'a str,
    /// TUI sink for per-sample stream events.
    pub tui: &'a SampleTui,
}

impl fmt::Debug for RunSampleBatch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunSampleBatch")
            .field("command", self.command)
            .field("task", self.task)
            .field(DEBUG_RUN_INDEX, &self.run_index)
            .finish_non_exhaustive()
    }
}

/// Host-provided I/O operations used by the library run state machine.
#[expect(
    clippy::missing_errors_doc,
    clippy::too_many_arguments,
    reason = "hook methods share the orchestrator error contract and bridge external side effects"
)]
pub trait RunLifecycleHooks {
    /// GitHub task brief type returned by intake.
    type TaskBrief: RunTaskBrief;

    /// Loads the required runtime project config.
    fn load_required_project_config(
        &self,
        project: &ProjectLayout,
    ) -> Result<ProjectConfig, AppError>;

    /// Validates the target repository before a run starts.
    fn preflight_target_repository(&self, config: &ProjectConfig) -> Result<(), AppError>;

    /// Validates the runtime repository before a run starts.
    fn preflight_runtime_repository(
        &self,
        project: &ProjectLayout,
        config: &ProjectConfig,
    ) -> Result<(), AppError>;

    /// Loads the GitHub issue or PR used as the task brief.
    fn load_github_task(
        &self,
        config: &ProjectConfig,
        task_id: &TaskId,
    ) -> Result<Self::TaskBrief, AppError>;

    /// Validates intake and posts a blocking comment when the brief is incomplete.
    fn post_intake_blocker_comment(
        &self,
        command: &RunCommand,
        config: &ProjectConfig,
        brief: &Self::TaskBrief,
    ) -> Result<Option<String>, AppError>;

    /// Publishes a runtime checkpoint and returns the pushed tag.
    fn publish_runtime_checkpoint(
        &self,
        project: &ProjectLayout,
        config: &ProjectConfig,
        task_id: &TaskId,
        label: &str,
    ) -> Result<String, AppError>;

    /// Ensures the runtime prompt exists after resume.
    fn ensure_runtime_prompt(&self, task: &TaskLayout, prompts: &PromptSet)
    -> Result<(), AppError>;

    /// Loads the runtime prompt to use for a resumed or new run.
    fn runtime_prompt_for_resume(
        &self,
        task: &TaskLayout,
        prompts: &PromptSet,
        next_run_index: usize,
    ) -> Result<String, AppError>;

    /// Handles resume from a selected sample or promotion state.
    fn resume_selected_sample_state(
        &self,
        state_lines: &mut Vec<String>,
        context: &RunContext<'_>,
        current_prompt: &mut String,
        resume_plan: &ResumePlan,
        next_run_index: &mut usize,
    ) -> Result<Option<String>, AppError>;

    /// Handles resume from delivery PR states.
    fn resume_delivery_pr_state(
        &self,
        state_lines: &mut Vec<String>,
        context: &RunContext<'_>,
        current_prompt: &mut String,
        resume_plan: &ResumePlan,
    ) -> Result<Option<String>, AppError>;

    /// Loads or initializes `result.json` for the run.
    fn task_result_for_resume(
        &self,
        task: &TaskLayout,
        resume_plan: &ResumePlan,
    ) -> Result<TaskResult, AppError>;

    /// Runs isolated samples for one outer run.
    fn run_samples(&self, batch: &RunSampleBatch<'_>) -> Result<Vec<SampleRunOutcome>, AppError>;

    /// Appends the selected sample to the task result.
    fn record_task_result_run(
        &self,
        task_result: &mut TaskResult,
        run_index: usize,
        selected: &SampleRunOutcome,
        task: &TaskLayout,
    );

    /// Promotes the selected sample into the durable best candidate.
    fn promote_selected_sample(
        &self,
        task: &TaskLayout,
        selected: &SampleRunOutcome,
    ) -> Result<(), AppError>;

    /// Pushes the promoted best and opens or updates the delivery PR.
    fn push_and_open_pr(
        &self,
        task: &TaskLayout,
        selected: &SampleRunOutcome,
        config: &ProjectConfig,
        task_readme: &str,
    ) -> Result<String, AppError>;

    /// Handles the PR feedback/readiness branch after opening the delivery PR.
    fn continue_after_opened_pr(
        &self,
        state_lines: &mut Vec<String>,
        context: &RunContext<'_>,
        current_prompt: &mut String,
        pr: &str,
        run_index: usize,
    ) -> Result<PrContinuation, AppError>;

    /// Evolves the runtime prompt after a rejected selected sample.
    fn evolve_prompt(
        &self,
        task: &TaskLayout,
        prompts: &PromptSet,
        current_prompt: &str,
        selected: &SampleRunOutcome,
        next_prompt_index: usize,
    ) -> Result<String, AppError>;
}

/// Library-owned run lifecycle coordinator.
pub struct RunOrchestrator<'a, H>
where
    H: RunLifecycleHooks + ?Sized,
{
    hooks: &'a H,
    outer_run_limit: usize,
}

impl<H> fmt::Debug for RunOrchestrator<'_, H>
where
    H: RunLifecycleHooks + ?Sized,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunOrchestrator")
            .field(DEBUG_OUTER_RUN_LIMIT, &self.outer_run_limit)
            .finish_non_exhaustive()
    }
}

impl<'a, H> RunOrchestrator<'a, H>
where
    H: RunLifecycleHooks + ?Sized,
{
    /// Creates a run orchestrator.
    #[must_use]
    pub const fn new(hooks: &'a H, outer_run_limit: usize) -> Self {
        Self {
            hooks,
            outer_run_limit,
        }
    }

    /// Executes the full run lifecycle until ready, continued, or terminal.
    ///
    /// # Errors
    ///
    /// Returns an error when a dependency, runtime, verifier, promotion, or
    /// delivery operation fails.
    #[expect(
        clippy::too_many_lines,
        reason = "the library state machine keeps the PRD lifecycle linear and auditable"
    )]
    pub fn execute(
        &self,
        invocation: &Invocation,
        command: &RunCommand,
    ) -> Result<String, AppError> {
        let judge_identity = JudgeIdentity::for_worker(command.agent());
        let plan = DependencyPlan::for_run(command.agent(), judge_identity.agent());
        let checker = DependencyChecker::new(SystemCommandProbe);
        let dependency_report = checker.check(&plan)?;
        let home = HomeResolver::resolve()?;
        let project = home.project(command.project_id().clone());
        let _lock = ProjectLock::acquire(&project)?;
        let config = self.hooks.load_required_project_config(&project)?;
        self.hooks.preflight_target_repository(&config)?;
        self.hooks.preflight_runtime_repository(&project, &config)?;
        let task_layout = project.task(command.task_id().clone());
        let resume_plan = ResumePlanner::plan(&task_layout)?;
        let mut state_lines = dependency_check_lines(command, &dependency_report);
        state_lines.push(format!(
            "cyanos: project={} task={} state=judge_config worker_agent={} judge_agent={} judge_model={} judge_isolation={}",
            command.project_id().as_str(),
            command.task_id().as_str(),
            command.agent().as_str(),
            judge_identity.agent().as_str(),
            judge_identity.model().as_label(),
            judge_identity.isolation_mode()
        ));
        if resume_plan.is_resume() {
            let pr = resume_plan
                .delivery_pr
                .as_ref()
                .map_or("none", |identity| identity.url.as_str());
            state_lines.push(format!(
                "cyanos: project={} task={} state=resuming from={} last_run={} next_run={} pr={pr}",
                command.project_id().as_str(),
                command.task_id().as_str(),
                resume_plan.state.as_str(),
                resume_plan.last_run_index,
                resume_plan.next_run_index
            ));
        }
        state_lines.push(format!(
            "cyanos: project={} task={} state=loading_task repo={}",
            command.project_id().as_str(),
            command.task_id().as_str(),
            config.repo()
        ));
        let brief = self.hooks.load_github_task(&config, command.task_id())?;
        if let Some(comment) = self
            .hooks
            .post_intake_blocker_comment(command, &config, &brief)?
        {
            state_lines.push(format!(
                "cyanos: project={} task={} state=blocked_intake comment={comment}",
                command.project_id().as_str(),
                command.task_id().as_str()
            ));
            let tag = self.hooks.publish_runtime_checkpoint(
                &project,
                &config,
                command.task_id(),
                "blocked-intake",
            )?;
            state_lines.push(format!(
                "cyanos: project={} task={} state=runtime_pushed tag={tag}",
                command.project_id().as_str(),
                command.task_id().as_str()
            ));
            return Ok(state_lines.join("\n"));
        }

        let request = crate::TaskRunRequest::new(
            home,
            command.project_id().clone(),
            command.task_id().clone(),
        );
        let runner = TaskRunner::new(SystemTaskGitRunner);
        let task_readme = brief.render_task_source();
        let report = runner.bootstrap_task(
            &request,
            brief.source_label(),
            task_readme.clone(),
            command.sample_count(),
        )?;
        let prompts = crate::EmbeddedPromptLoader::load();
        if resume_plan.is_resume() {
            self.hooks.ensure_runtime_prompt(report.task(), &prompts)?;
        } else {
            RuntimeCheckpoint::append_ledger(
                report.task(),
                STATE_BRIEF_FROZEN,
                "task runtime bootstrapped",
            )?;
            RuntimePromptWriter::write_initial(report.task(), &prompts)?;
        }
        let mut next_run_index = resume_plan.next_run_index;
        let mut current_prompt =
            self.hooks
                .runtime_prompt_for_resume(report.task(), &prompts, next_run_index)?;
        let context = RunContext::new(
            invocation,
            &project,
            &config,
            command,
            report.task(),
            &prompts,
            &task_readme,
        );
        if let Some(response) = self.hooks.resume_selected_sample_state(
            &mut state_lines,
            &context,
            &mut current_prompt,
            &resume_plan,
            &mut next_run_index,
        )? {
            return Ok(response);
        }
        if let Some(pr_response) = self.hooks.resume_delivery_pr_state(
            &mut state_lines,
            &context,
            &mut current_prompt,
            &resume_plan,
        )? {
            return Ok(pr_response);
        }
        if next_run_index > self.outer_run_limit {
            return Err(AgentRuntimeError::new(format!(
                "task {} resumed from {} after {} run(s), but no outer runs remain; next action: inspect runtime result.json and ledger.jsonl, then resolve the terminal blocker before rerunning",
                command.task_id().as_str(),
                resume_plan.state.as_str(),
                resume_plan.last_run_index
            ))
            .into());
        }
        let mut task_result = self
            .hooks
            .task_result_for_resume(report.task(), &resume_plan)?;
        let mut final_summary = None;

        for run_index in next_run_index..=self.outer_run_limit {
            if run_index > 1 {
                runner.bootstrap_sample_run(&request, run_index, command.sample_count())?;
            }

            let prompt_label = format!("prompt.{}.md", run_index - 1);
            let tui = SampleTui::for_task(
                command.task_id().as_str(),
                run_index,
                command.sample_count(),
            );
            state_lines.push(format!(
                "cyanos: project={} task={} state=running_samples samples={}",
                command.project_id().as_str(),
                command.task_id().as_str(),
                command.sample_count()
            ));
            RuntimeCheckpoint::append_ledger(
                report.task(),
                STATE_RUNNING_SAMPLES,
                "sample execution started",
            )?;
            let outcomes = self.hooks.run_samples(&RunSampleBatch {
                command,
                task: report.task(),
                run_index,
                inner_prompt: &current_prompt,
                task_readme: &task_readme,
                tui: &tui,
            })?;
            tui.finish()
                .map_err(|source| AppError::io("finish sample TUI", source))?;
            let selected = select_sample(&outcomes)?;
            self.hooks
                .record_task_result_run(&mut task_result, run_index, selected, report.task());
            fs::write(report.task().result(), task_result.to_json())
                .map_err(|source| AppError::io("write task result", source))?;
            RuntimeCheckpoint::append_ledger(
                report.task(),
                STATE_EVALUATED_SAMPLES,
                &format!("selected run {run_index} sample {}", selected.sample_id),
            )?;
            state_lines.push(format!(
                "cyanos: project={} task={} state=evaluated_samples selected_sample={} score={}",
                command.project_id().as_str(),
                command.task_id().as_str(),
                selected.sample_id,
                selected.score.total().as_i64()
            ));
            let tag = self.hooks.publish_runtime_checkpoint(
                &project,
                &config,
                command.task_id(),
                &format!("sample-selected-run-{run_index}"),
            )?;
            state_lines.push(format!(
                "cyanos: project={} task={} state=runtime_pushed tag={tag}",
                command.project_id().as_str(),
                command.task_id().as_str()
            ));

            final_summary = Some(final_sample_summary(
                invocation,
                report.task(),
                prompt_label.as_str(),
                selected,
                None,
            ));

            if selected.evaluation.is_accepted() {
                RuntimeCheckpoint::append_ledger(
                    report.task(),
                    STATE_GLOBAL_BEST_ACCEPTED,
                    "selected sample accepted",
                )?;
                self.hooks
                    .promote_selected_sample(report.task(), selected)?;
                let tag = self.hooks.publish_runtime_checkpoint(
                    &project,
                    &config,
                    command.task_id(),
                    CHECKPOINT_BEST_PROMOTED,
                )?;
                state_lines.push(format!(
                    "cyanos: project={} task={} state=runtime_pushed tag={tag}",
                    command.project_id().as_str(),
                    command.task_id().as_str()
                ));
                let pr =
                    self.hooks
                        .push_and_open_pr(report.task(), selected, &config, &task_readme)?;
                if matches!(
                    self.hooks.continue_after_opened_pr(
                        &mut state_lines,
                        &context,
                        &mut current_prompt,
                        &pr,
                        run_index,
                    )?,
                    PrContinuation::Continue
                ) {
                    continue;
                }
                if let Some(summary) = final_summary.as_mut() {
                    summary.push_str(" pr=");
                    summary.push_str(&pr);
                }

                if let Some(summary) = final_summary {
                    state_lines.push(summary);
                    return Ok(state_lines.join("\n"));
                }

                return Err(
                    AgentRuntimeError::new("no sample outcomes were produced".to_owned()).into(),
                );
            }

            RuntimeCheckpoint::append_ledger(
                report.task(),
                STATE_GLOBAL_BEST_REJECTED,
                "selected sample rejected",
            )?;
            if run_index < self.outer_run_limit {
                current_prompt = self.hooks.evolve_prompt(
                    report.task(),
                    &prompts,
                    &current_prompt,
                    selected,
                    run_index,
                )?;
                RuntimeCheckpoint::append_ledger(
                    report.task(),
                    STATE_PROMPT_EVOLVED,
                    &format!("wrote prompt.{run_index}.md"),
                )?;
            }
        }

        let tag = self.hooks.publish_runtime_checkpoint(
            &project,
            &config,
            command.task_id(),
            "terminal-failure",
        )?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=runtime_pushed tag={tag}",
            command.project_id().as_str(),
            command.task_id().as_str()
        ));
        let summary = final_summary.ok_or_else(|| {
            AppError::from(AgentRuntimeError::new(
                "no sample outcomes were produced".to_owned(),
            ))
        })?;
        RuntimeCheckpoint::append_ledger(
            report.task(),
            STATE_TERMINAL_FAILURE,
            "outer runs exhausted without an accepted sample",
        )?;
        Err(AgentRuntimeError::new(format!(
            "task {} reached terminal failure after {} run(s); runtime checkpoint tag={tag}; {summary}; next action: inspect the verifier feedback and rerun Cyanos after fixing the task or implementation",
            command.task_id().as_str(),
            self.outer_run_limit
        ))
        .into())
    }
}

/// Selects the highest-scoring sample, preferring accepted samples and stable
/// lower sample ids on ties.
///
/// # Errors
///
/// Returns an error when no sample outcomes were produced.
pub fn select_sample(outcomes: &[SampleRunOutcome]) -> Result<&SampleRunOutcome, AppError> {
    outcomes
        .iter()
        .max_by_key(|outcome| {
            (
                outcome.evaluation.is_accepted(),
                outcome.score.total().as_i64(),
                std::cmp::Reverse(outcome.sample_id),
            )
        })
        .ok_or_else(|| {
            AppError::from(AgentRuntimeError::new(
                "no sample outcomes were produced".to_owned(),
            ))
        })
}

fn dependency_check_lines(command: &RunCommand, report: &crate::DependencyReport) -> Vec<String> {
    report
        .checks()
        .iter()
        .map(|check| {
            let attempts = if check.attempts() > 1 {
                format!(" attempts={}", check.attempts())
            } else {
                String::new()
            };
            format!(
                "cyanos: project={} task={} check=✅ name={}{}",
                command.project_id().as_str(),
                command.task_id().as_str(),
                RuntimeCheckpoint::sanitize_state_value(&check.requirement().label()),
                attempts
            )
        })
        .collect()
}

fn final_sample_summary(
    invocation: &Invocation,
    task: &TaskLayout,
    prompt_label: &str,
    selected: &SampleRunOutcome,
    pr: Option<&str>,
) -> String {
    let mut summary = format!(
        "{} dependencies=ok task={} prompt={} selected_sample={} score={} summary={} patch={} eval={}",
        invocation.render(),
        task.task_id().as_str(),
        prompt_label,
        selected.sample_id,
        selected.score.total().as_i64(),
        relative_label(task, &selected.summary_path),
        relative_label(task, &selected.patch_path),
        relative_label(task, &selected.eval_path)
    );
    if let Some(pr) = pr {
        summary.push_str(" pr=");
        summary.push_str(pr);
    }
    summary
}

fn relative_label(task: &TaskLayout, path: &std::path::Path) -> String {
    path.strip_prefix(task.root()).map_or_else(
        |_| path.display().to_string(),
        |relative| relative.display().to_string(),
    )
}

fn baseline_score() -> ScoreBreakdown {
    ScoreBreakdown::new(
        SampleScore::new(0),
        crate::QualityTier::CompileFailed,
        SampleScore::new(0),
        SampleScore::new(0),
        SampleScore::new(0),
    )
}

fn perfect_score() -> ScoreBreakdown {
    ScoreBreakdown::new(
        SampleScore::new(PERFECT_SAMPLE_SCORE),
        crate::QualityTier::Passed,
        SampleScore::new(PERFECT_SAMPLE_SCORE),
        SampleScore::new(PERFECT_SAMPLE_SCORE),
        SampleScore::new(PERFECT_SAMPLE_SCORE),
    )
}

/// Creates a new empty task result using the lifecycle scoring baseline.
#[must_use]
pub fn new_task_result() -> TaskResult {
    TaskResult::new(baseline_score(), perfect_score())
}
