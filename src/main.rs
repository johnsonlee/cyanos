//! Command-line entry point for Cyanos.

use std::{
    fmt::Write as _,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{ExitCode, Output},
    thread,
    time::Duration,
};

const CARGO_COVERAGE_ARGS: &[&str] = &[
    "llvm-cov",
    "--summary-only",
    "--all-targets",
    "--all-features",
];
const CARGO_PROGRAM: &str = "cargo";
const CYANOS_COMMENT_MARKER_TASK_PREFIX: &str = "<!--cyanos:task=";
const CYANOS_COMMENT_MARKER_TASK_INTAKE: &str = "<!-- cyanos:source=task-intake -->";
const CYANOS_COMMENT_MARKER_SUFFIX: &str = "-->";
const FORCE_COVERAGE_VERIFIER_ENV: &str = "CYANOS_FORCE_COVERAGE_VERIFIER";
const GH_PROGRAM: &str = "gh";
const GIT_DIR: &str = ".git";
const GIT_SUFFIX: &str = ".git";
const GITIGNORE_FILE: &str = ".gitignore";
const GIT_INIT_ARG: &str = "init";
const GIT_ORIGIN_HEAD_REF: &str = "origin/HEAD";
const GIT_PROGRAM: &str = "git";
const GIT_ORIGIN_REMOTE: &str = "origin";
const GIT_PATHSPEC_ALL: &str = ".";
const GIT_PATHSPEC_EXCLUDE_PROFRAW: &str = ":(exclude)*.profraw";
const GIT_PATHSPEC_EXCLUDE_TARGET: &str = ":(exclude)target";
const GIT_STATUS_ARG: &str = "status";
const GIT_STATUS_PORCELAIN_ARG: &str = "--porcelain=v1";
const GIT_STATUS_UNTRACKED_ARG: &str = "--untracked-files=all";
const GIT_WORKTREE_ARG: &str = "worktree";
const GITHUB_OBJECT_ISSUE: &str = "Issue";
const GITHUB_OBJECT_PULL_REQUEST: &str = "PullRequest";
const GITHUB_TASK_TYPE_JQ: &str = r#".data.repository.issueOrPullRequest.__typename // """#;
const GITHUB_TASK_TYPE_QUERY: &str = "query($owner:String!, $repo:String!, $number:Int!) { repository(owner:$owner, name:$repo) { issueOrPullRequest(number:$number) { __typename } } }";
const LLVM_PROFILE_FILE_ENV: &str = "LLVM_PROFILE_FILE";
const MIN_TASK_SECTION_CHARS: usize = 16;
const OUTER_RUN_LIMIT: usize = 3;
const PERFECT_SAMPLE_SCORE: i64 = 10_000;
const PR_READINESS_JSON_FIELDS: &str = "reviewDecision,mergeStateStatus,statusCheckRollup";
const PR_LIST_JSON_FIELDS: &str = "number,url,title,body,headRefName";
const PR_REVIEW_THREADS_QUERY: &str = "query($owner:String!, $repo:String!, $number:Int!) { repository(owner:$owner, name:$repo) { pullRequest(number:$number) { reviewThreads(first:100) { nodes { id isResolved isOutdated path comments(first:20) { nodes { databaseId body url author { login } } } } } } } }";
const PR_REVIEW_THREADS_TSV_JQ: &str = r#".data.repository.pullRequest.reviewThreads.nodes[] as $thread | $thread.comments.nodes[] | [$thread.id, ($thread.isResolved | tostring), ($thread.isOutdated | tostring), ($thread.path // ""), (.databaseId | tostring), (.author.login // "unknown"), (.url // ""), (.body // "" | gsub("[\t\r\n]"; " "))] | @tsv"#;
const PR_REVIEW_THREAD_RESOLVE_MUTATION: &str = "mutation($thread:ID!) { resolveReviewThread(input:{threadId:$thread}) { thread { id isResolved } } }";
const VIEWER_LOGIN_JQ: &str = ".data.viewer.login";
const VIEWER_LOGIN_QUERY: &str = "query { viewer { login } }";
const RETRY_ATTEMPTS: usize = 3;
const RETRY_DELAY: Duration = Duration::from_millis(50);
const RUNTIME_BRANCH: &str = "main";
const RUNTIME_GIT_EMAIL: &str = "cyanos@example.invalid";
const RUNTIME_GITIGNORE: &str =
    "origin/\ncyanos.lock\ntasks/*/worktree/\ntasks/*/runs/*/samples/*/worktree/\n";
const RUNTIME_PROMPT_FILE: &str = "prompt.md";
const TASK_SECTION_MAX_UNGROUPED_BULLETS: usize = 5;
const CHECKPOINT_BEST_PROMOTED: &str = "best-promoted";
#[cfg(test)]
const STATE_BRIEF_FROZEN: &str = "brief_frozen";
#[cfg(test)]
const STATE_EVALUATED_SAMPLES: &str = "evaluated_samples";
const STATE_GLOBAL_BEST_ACCEPTED: &str = "global_best_accepted";
const STATE_GLOBAL_BEST_REJECTED: &str = "global_best_rejected";
const STATE_PR_FEEDBACK: &str = "pr_feedback";
const STATE_PR_OPENED: &str = "pr_opened";
const STATE_PR_READY: &str = "pr_ready";
const STATE_PROMPT_EVOLVED: &str = "prompt_evolved";
#[cfg(test)]
const STATE_RUNNING_SAMPLES: &str = "running_samples";
const STATE_RUNTIME_PUBLISH_FAILED: &str = "runtime_publish_failed";
const STATE_TERMINAL_FAILURE: &str = "terminal_failure";
const TASK_BODY_MARKER: &str = "---CYANOS-BODY---";
const TASK_TITLE_MARKER: &str = "---CYANOS-TITLE---";
const UNKNOWN_REPOSITORY: &str = "unknown/unknown";
const REQUIRED_TASK_SECTIONS: [&str; 5] = [
    "Problem",
    "Expected Behavior",
    "Scope",
    "Acceptance Criteria",
    "Verification",
];
const VERIFICATION_FAILURE_CLASS: &str = "verification";

fn main() -> ExitCode {
    let terminal = Terminal::new();
    let cli = cyanos::CyanosCli::default();

    terminal.exit(
        cli.parse(std::env::args().skip(1))
            .map_err(cyanos::AppError::from),
    )
}

struct Terminal {
    stdout: io::Stdout,
    stderr: io::Stderr,
    error_reporter: cyanos::ErrorReporter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReviewResolutionEvidence {
    commit: String,
    eval: String,
    fixed_thread_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeReviewThreadEvidence {
    eval: String,
    fixed_thread_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PullRequestReviewTarget<'a> {
    host: Option<&'a str>,
    owner: &'a str,
    repo: &'a str,
    number: &'a str,
    pr: &'a str,
}

#[derive(Clone, Copy, Debug)]
struct FixedReviewThreadResolution<'a> {
    records: &'a [cyanos::ReviewCommentRecord],
    managed_actor: &'a str,
    thread_ids: &'a [String],
    evidence: &'a ReviewResolutionEvidence,
}

impl Terminal {
    fn new() -> Self {
        Self {
            stdout: io::stdout(),
            stderr: io::stderr(),
            error_reporter: cyanos::ErrorReporter::default(),
        }
    }

    fn exit(&self, result: Result<cyanos::Invocation, cyanos::AppError>) -> ExitCode {
        match result {
            Ok(invocation) => self.execute(&invocation),
            Err(error) => self.exit_with_error(&error),
        }
    }

    fn execute(&self, invocation: &cyanos::Invocation) -> ExitCode {
        let result = match invocation.command() {
            cyanos::CliCommand::Help | cyanos::CliCommand::Version => Ok(invocation.render()),
            cyanos::CliCommand::Init(command) => self.execute_init(command),
            cyanos::CliCommand::Run(command) => self.execute_run(invocation, command),
        };

        match result {
            Ok(response) => self.exit_with_output(&response),
            Err(error) => self.exit_with_error(&error),
        }
    }

    fn execute_init(&self, command: &cyanos::InitCommand) -> Result<String, cyanos::AppError> {
        let project_id = match command.project_id() {
            Some(project_id) => project_id.clone(),
            None => self.prompt_project_id()?,
        };
        let source_path = current_dir()?;
        let home = cyanos::HomeResolver::resolve()?;
        let request = cyanos::ProjectInitRequest::new(home, project_id, source_path);
        let initializer = cyanos::ProjectInitializer::new(cyanos::SystemGitRunner);
        let report = initializer.initialize(&request)?;

        Ok(format!(
            "cyanos: initialized project={} origin={} repo={} runtime_repo={} base_branch={} task_readme={}",
            report.project().project_id().as_str(),
            report.project().origin().display(),
            report.config().repo(),
            report.config().runtime_repo(),
            report.config().base_branch(),
            report.wrote_task_readme()
        ))
    }

    fn execute_run(
        &self,
        invocation: &cyanos::Invocation,
        command: &cyanos::RunCommand,
    ) -> Result<String, cyanos::AppError> {
        cyanos::RunOrchestrator::new(self, OUTER_RUN_LIMIT).execute(invocation, command)
    }

    fn run_samples(batch: &SampleBatch<'_>) -> Result<Vec<SampleRunOutcome>, cyanos::AppError> {
        let mut outcomes = Vec::with_capacity(batch.command.sample_count());

        thread::scope(|scope| -> Result<(), cyanos::AppError> {
            let mut handles = Vec::with_capacity(batch.command.sample_count());

            for sample_id in 1..=batch.command.sample_count() {
                let agent = batch.command.agent();
                let model = batch.command.model().clone();
                let cwd = batch.task.sample_run_worktree(sample_id, batch.run_index);
                let summary_path = batch.task.sample_summary(sample_id, batch.run_index);
                let eval_path = batch.task.sample_eval(sample_id, batch.run_index);
                let patch_path = batch.task.sample_patch(sample_id, batch.run_index);
                let inner_prompt = batch.inner_prompt.to_owned();
                let task_readme = batch.task_readme.to_owned();
                let command = batch.command;
                let mut observer = batch.tui.observer(sample_id);

                handles.push(scope.spawn(move || {
                    if let Some(parent) = summary_path.parent() {
                        fs::create_dir_all(parent).map_err(|source| {
                            cyanos::AppError::io("create sample artifact directory", source)
                        })?;
                    }

                    let mut runtime = cyanos::SystemAgentRuntime::new(
                        agent,
                        model,
                        cwd.clone(),
                        cyanos::SystemAgentCommandRunner,
                    );
                    let turn = cyanos::AgentTurn::new(inner_prompt, task_readme.clone());
                    let output = Self::execute_sample_agent_with_retry(
                        &mut runtime,
                        &cwd,
                        &turn,
                        &mut observer,
                    )?;
                    let verification =
                        Self::evaluate_sample_worktree_for_command(&cwd, &task_readme, command)?;
                    let (evaluation, score) = verification.into_parts();
                    Self::write_sample_patch(&cwd, &patch_path)?;
                    fs::write(&summary_path, output.content())
                        .map_err(|source| cyanos::AppError::io("write sample summary", source))?;
                    fs::write(&eval_path, evaluation.to_json())
                        .map_err(|source| cyanos::AppError::io("write sample eval", source))?;

                    Ok::<SampleRunOutcome, cyanos::AppError>(SampleRunOutcome {
                        run_index: batch.run_index,
                        sample_id,
                        evaluation,
                        score,
                        eval_path,
                        summary_path,
                        patch_path,
                    })
                }));
            }

            for handle in handles {
                let outcome = handle.join().map_err(|_panic| {
                    cyanos::AppError::from(cyanos::AgentRuntimeError::new(
                        "sample worker panicked".to_owned(),
                    ))
                })??;
                batch
                    .tui
                    .finish_sample(
                        outcome.sample_id,
                        outcome.evaluation.is_accepted(),
                        outcome.score.total().as_i64(),
                    )
                    .map_err(|source| cyanos::AppError::io("update sample TUI", source))?;
                outcomes.push(outcome);
            }

            Ok(())
        })?;

        outcomes.sort_by_key(|outcome| outcome.sample_id);
        Ok(outcomes)
    }

    #[cfg(test)]
    fn evaluate_sample_worktree(
        worktree: &Path,
    ) -> Result<cyanos::SampleVerification, cyanos::AppError> {
        Self::evaluate_sample_worktree_with_coverage(worktree, Self::measure_coverage)
    }

    fn evaluate_sample_worktree_for_command(
        worktree: &Path,
        task_readme: &str,
        command: &cyanos::RunCommand,
    ) -> Result<cyanos::SampleVerification, cyanos::AppError> {
        Self::evaluate_sample_worktree_with_verifiers(
            worktree,
            task_readme,
            Self::measure_coverage,
            |worktree, prompt| Self::run_llm_judge(command, worktree, prompt),
        )
    }

    fn command_output(
        program: &str,
        worktree: &Path,
        args: &[&str],
        action: &'static str,
    ) -> Result<Output, cyanos::AppError> {
        cyanos::process::output(program, worktree, args)
            .map_err(|source| cyanos::AppError::io(action, source))
    }

    fn command_output_retrying_errors(
        program: &str,
        worktree: &Path,
        args: &[&str],
        action: &'static str,
    ) -> Result<Output, cyanos::AppError> {
        let mut last_error = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            match Self::command_output(program, worktree, args, action) {
                Ok(output) => return Ok(output),
                Err(error) => {
                    last_error = Some(error);
                    thread::sleep(RETRY_DELAY);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            cyanos::AgentRuntimeError::new(format!("{action} failed without an error")).into()
        }))
    }

    fn git_output_retrying_errors(
        worktree: &Path,
        args: &[&str],
        action: &'static str,
    ) -> Result<Output, cyanos::AppError> {
        Self::command_output_retrying_errors(GIT_PROGRAM, worktree, args, action)
    }

    #[cfg(test)]
    fn evaluate_sample_worktree_with_coverage(
        worktree: &Path,
        measure_coverage: fn(&Path) -> Result<i64, cyanos::AppError>,
    ) -> Result<cyanos::SampleVerification, cyanos::AppError> {
        Self::evaluate_sample_worktree_with_verifiers(
            worktree,
            "",
            measure_coverage,
            |_worktree, _prompt| Ok(cyanos::JudgeReview::passed("unit judge", Vec::new())),
        )
    }

    fn evaluate_sample_worktree_with_verifiers<M, J>(
        worktree: &Path,
        task_readme: &str,
        measure_coverage: M,
        judge_review: J,
    ) -> Result<cyanos::SampleVerification, cyanos::AppError>
    where
        M: Fn(&Path) -> Result<i64, cyanos::AppError>,
        J: Fn(&Path, &str) -> Result<cyanos::JudgeReview, cyanos::AppError>,
    {
        cyanos::evaluate_sample_worktree_with_verifiers(
            worktree,
            task_readme,
            Self::command_output,
            measure_coverage,
            judge_review,
        )
    }

    fn execute_agent_with_retry(
        runtime: &mut cyanos::SystemAgentRuntime<cyanos::SystemAgentCommandRunner>,
        turn: &cyanos::AgentTurn,
        observer: &mut dyn cyanos::AgentStreamObserver,
    ) -> Result<cyanos::AgentOutput, cyanos::AppError> {
        let mut last_error = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            match cyanos::AgentRuntime::execute_with_observer(runtime, turn, observer) {
                Ok(output) => return Ok(output),
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error.map_or_else(
            || {
                cyanos::AppError::from(cyanos::AgentRuntimeError::new(
                    "agent execution failed without an error".to_owned(),
                ))
            },
            cyanos::AppError::from,
        ))
    }

    fn execute_sample_agent_with_retry(
        runtime: &mut cyanos::SystemAgentRuntime<cyanos::SystemAgentCommandRunner>,
        worktree: &Path,
        turn: &cyanos::AgentTurn,
        observer: &mut dyn cyanos::AgentStreamObserver,
    ) -> Result<cyanos::AgentOutput, cyanos::AppError> {
        let base_ref = Self::current_head(worktree)?;
        let mut last_error = None;
        for attempt in 1..=RETRY_ATTEMPTS {
            if attempt > 1 {
                Self::reset_sample_worktree(worktree, &base_ref)?;
            }
            match cyanos::AgentRuntime::execute_with_observer(runtime, turn, observer) {
                Ok(output) => return Ok(output),
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error.map_or_else(
            || {
                cyanos::AppError::from(cyanos::AgentRuntimeError::new(
                    "agent execution failed without an error".to_owned(),
                ))
            },
            cyanos::AppError::from,
        ))
    }

    fn current_head(worktree: &Path) -> Result<String, cyanos::AppError> {
        let output = Self::git_output(worktree, &["rev-parse", "HEAD"], "capture sample base")?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "git rev-parse HEAD failed in {} with exit code {:?}",
            worktree.display(),
            output.status.code()
        ))
        .into())
    }

    fn reset_sample_worktree(worktree: &Path, base_ref: &str) -> Result<(), cyanos::AppError> {
        Self::run_git(
            worktree,
            &["reset", "--hard", base_ref],
            "reset sample worktree before retry",
        )?;
        Self::run_git(
            worktree,
            &["clean", "-fdx"],
            "clean sample worktree before retry",
        )
    }

    #[cfg(test)]
    fn select_sample(outcomes: &[SampleRunOutcome]) -> Result<&SampleRunOutcome, cyanos::AppError> {
        cyanos::select_sample(outcomes)
    }

    #[cfg(test)]
    fn dependency_check_lines(
        command: &cyanos::RunCommand,
        report: &cyanos::DependencyReport,
    ) -> Vec<String> {
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
                    cyanos::RuntimeCheckpoint::sanitize_state_value(&check.requirement().label()),
                    attempts
                )
            })
            .collect()
    }

    #[cfg(test)]
    fn new_task_result() -> cyanos::TaskResult {
        cyanos::new_task_result()
    }

    fn task_result_for_resume(
        task: &cyanos::TaskLayout,
        resume_plan: &cyanos::ResumePlan,
    ) -> Result<cyanos::TaskResult, cyanos::AppError> {
        if resume_plan.last_run_index == 0 {
            return Ok(cyanos::new_task_result());
        }

        match fs::read_to_string(task.result()) {
            Ok(content) => cyanos::TaskResult::from_json(&content).map_err(|source| {
                cyanos::AppError::io(
                    "parse task result for resume",
                    io::Error::new(io::ErrorKind::InvalidData, source),
                )
            }),
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                Err(cyanos::AgentRuntimeError::new(format!(
                    "task {} resume planned after {} run(s), but result.json is missing; next action: inspect runtime ledger and restore result.json before rerunning",
                    task.task_id().as_str(),
                    resume_plan.last_run_index
                ))
                .into())
            }
            Err(source) => Err(cyanos::AppError::io("read task result for resume", source)),
        }
    }

    #[cfg(test)]
    fn plan_resume(task: &cyanos::TaskLayout) -> Result<cyanos::ResumePlan, cyanos::AppError> {
        cyanos::ResumePlanner::plan(task)
    }

    fn ensure_runtime_prompt(
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
    ) -> Result<(), cyanos::AppError> {
        if task.root().join(RUNTIME_PROMPT_FILE).is_file() {
            return Ok(());
        }
        cyanos::RuntimePromptWriter::write_initial(task, prompts)?;
        Ok(())
    }

    fn runtime_prompt_for_resume(
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        next_run_index: usize,
    ) -> Result<String, cyanos::AppError> {
        let runtime_prompt = task.root().join(RUNTIME_PROMPT_FILE);
        if runtime_prompt.is_file() {
            return fs::read_to_string(runtime_prompt)
                .map_err(|source| cyanos::AppError::io("read runtime prompt", source));
        }
        if next_run_index > 1 {
            let snapshot = task.prompt_snapshot(next_run_index - 1);
            if snapshot.is_file() {
                return fs::read_to_string(snapshot).map_err(|source| {
                    cyanos::AppError::io("read runtime prompt snapshot", source)
                });
            }
        }
        Ok(prompts.inner().as_str().to_owned())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "selected sample resume has to bridge persisted task, project, prompt, and output state"
    )]
    fn resume_selected_sample_state(
        state_lines: &mut Vec<String>,
        invocation: &cyanos::Invocation,
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
        command: &cyanos::RunCommand,
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        current_prompt: &mut String,
        resume_plan: &cyanos::ResumePlan,
        task_readme: &str,
        next_run_index: &mut usize,
    ) -> Result<Option<String>, cyanos::AppError> {
        if !matches!(
            resume_plan.state,
            cyanos::ResumeState::SampleEvaluation | cyanos::ResumeState::Promotion
        ) {
            return Ok(None);
        }
        if resume_plan.last_run_index == 0 {
            return Ok(None);
        }

        let run_index = resume_plan.last_run_index;
        let selected = Self::selected_outcome_for_resume(task, run_index)?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=resumed_selected_sample run={} selected_sample={} score={}",
            command.project_id().as_str(),
            command.task_id().as_str(),
            run_index,
            selected.sample_id,
            selected.score.total().as_i64()
        ));

        if selected.evaluation.is_accepted() {
            return Self::resume_accepted_selected_sample(
                state_lines,
                invocation,
                project,
                config,
                command,
                task,
                prompts,
                current_prompt,
                resume_plan,
                task_readme,
                next_run_index,
                &selected,
                run_index,
            );
        }
        if matches!(resume_plan.state, cyanos::ResumeState::Promotion) {
            return Err(cyanos::AgentRuntimeError::new(format!(
                "task {} resumed promotion for run {run_index}, but persisted selected eval is rejected; next action: inspect result.json and eval.json before rerunning",
                command.task_id().as_str()
            ))
            .into());
        }

        Self::resume_rejected_selected_sample(
            state_lines,
            project,
            config,
            command,
            task,
            prompts,
            current_prompt,
            next_run_index,
            &selected,
            run_index,
        )?;
        Ok(None)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "PR continuation needs checkpoint, prompt, command, and output context"
    )]
    fn continue_after_opened_pr(
        state_lines: &mut Vec<String>,
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
        command: &cyanos::RunCommand,
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        current_prompt: &mut String,
        pr: &str,
        run_index: usize,
    ) -> Result<PrContinuation, cyanos::AppError> {
        cyanos::RuntimeCheckpoint::append_ledger(task, STATE_PR_OPENED, pr)?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=pr_opened pr={pr}",
            command.project_id().as_str(),
            command.task_id().as_str()
        ));
        let tag =
            Self::publish_runtime_checkpoint(project, config, command.task_id(), "pr-opened")?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=runtime_pushed tag={tag}",
            command.project_id().as_str(),
            command.task_id().as_str()
        ));
        if let Some(feedback) = Self::pr_readiness_feedback(pr, config)? {
            cyanos::RuntimeCheckpoint::append_ledger(task, STATE_PR_FEEDBACK, &feedback)?;
            state_lines.push(format!(
                "cyanos: project={} task={} state=pr_feedback summary={}",
                command.project_id().as_str(),
                command.task_id().as_str(),
                cyanos::RuntimeCheckpoint::sanitize_state_value(&feedback)
            ));
            let tag = Self::publish_runtime_checkpoint(
                project,
                config,
                command.task_id(),
                &format!("pr-feedback-run-{run_index}"),
            )?;
            state_lines.push(format!(
                "cyanos: project={} task={} state=runtime_pushed tag={tag}",
                command.project_id().as_str(),
                command.task_id().as_str()
            ));
            if run_index < OUTER_RUN_LIMIT {
                *current_prompt = Self::evolve_prompt_from_feedback(
                    task,
                    prompts,
                    current_prompt,
                    &feedback,
                    run_index,
                )?;
                cyanos::RuntimeCheckpoint::append_ledger(
                    task,
                    STATE_PROMPT_EVOLVED,
                    &format!("wrote prompt.{run_index}.md from PR feedback"),
                )?;
                return Ok(PrContinuation::Continue);
            }
            let tag = Self::publish_runtime_checkpoint(
                project,
                config,
                command.task_id(),
                "terminal-failure",
            )?;
            cyanos::RuntimeCheckpoint::append_ledger(
                task,
                STATE_TERMINAL_FAILURE,
                "PR readiness feedback remained unresolved",
            )?;
            return Err(cyanos::AgentRuntimeError::new(format!(
                "task {} reached terminal failure after {} run(s); PR readiness still has unresolved feedback for {pr}; runtime checkpoint tag={tag}; feedback={}; next action: inspect the PR checks/review comments and rerun Cyanos after addressing the blocker",
                command.task_id().as_str(),
                OUTER_RUN_LIMIT,
                cyanos::RuntimeCheckpoint::sanitize_state_value(&feedback)
            ))
            .into());
        }

        let tag = Self::publish_runtime_checkpoint(project, config, command.task_id(), "pr-ready")?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=runtime_pushed tag={tag}",
            command.project_id().as_str(),
            command.task_id().as_str()
        ));
        cyanos::RuntimeCheckpoint::append_ledger(task, STATE_PR_READY, "ready for user merge")?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=pr_ready pr={pr}",
            command.project_id().as_str(),
            command.task_id().as_str()
        ));
        Ok(PrContinuation::Ready)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "accepted resume mirrors the accepted delivery branch of the run loop"
    )]
    fn resume_accepted_selected_sample(
        state_lines: &mut Vec<String>,
        invocation: &cyanos::Invocation,
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
        command: &cyanos::RunCommand,
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        current_prompt: &mut String,
        resume_plan: &cyanos::ResumePlan,
        task_readme: &str,
        next_run_index: &mut usize,
        selected: &SampleRunOutcome,
        run_index: usize,
    ) -> Result<Option<String>, cyanos::AppError> {
        if matches!(resume_plan.state, cyanos::ResumeState::SampleEvaluation) {
            cyanos::RuntimeCheckpoint::append_ledger(
                task,
                STATE_GLOBAL_BEST_ACCEPTED,
                "resumed selected sample accepted",
            )?;
        }
        Self::promote_selected_sample(task, selected)?;
        let tag = Self::publish_runtime_checkpoint(
            project,
            config,
            command.task_id(),
            CHECKPOINT_BEST_PROMOTED,
        )?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=runtime_pushed tag={tag}",
            command.project_id().as_str(),
            command.task_id().as_str()
        ));
        let pr = Self::push_and_open_pr(task, selected, config, task_readme)?;
        if matches!(
            Self::continue_after_opened_pr(
                state_lines,
                project,
                config,
                command,
                task,
                prompts,
                current_prompt,
                &pr,
                run_index,
            )?,
            PrContinuation::Continue
        ) {
            *next_run_index = run_index.saturating_add(1);
            return Ok(None);
        }
        state_lines.push(Self::final_sample_summary(
            invocation,
            task,
            run_index,
            selected,
            Some(pr.as_str()),
        ));
        Ok(Some(state_lines.join("\n")))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "rejected resume has to update prompt state and checkpoint output"
    )]
    fn resume_rejected_selected_sample(
        state_lines: &mut Vec<String>,
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
        command: &cyanos::RunCommand,
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        current_prompt: &mut String,
        next_run_index: &mut usize,
        selected: &SampleRunOutcome,
        run_index: usize,
    ) -> Result<(), cyanos::AppError> {
        cyanos::RuntimeCheckpoint::append_ledger(
            task,
            STATE_GLOBAL_BEST_REJECTED,
            "resumed selected sample rejected",
        )?;
        if run_index < OUTER_RUN_LIMIT {
            *current_prompt =
                Self::evolve_prompt(task, prompts, current_prompt, selected, run_index)?;
            cyanos::RuntimeCheckpoint::append_ledger(
                task,
                STATE_PROMPT_EVOLVED,
                &format!("wrote prompt.{run_index}.md from resumed selected sample"),
            )?;
            *next_run_index = run_index.saturating_add(1);
            return Ok(());
        }

        let tag = Self::publish_runtime_checkpoint(
            project,
            config,
            command.task_id(),
            "terminal-failure",
        )?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=runtime_pushed tag={tag}",
            command.project_id().as_str(),
            command.task_id().as_str()
        ));
        cyanos::RuntimeCheckpoint::append_ledger(
            task,
            STATE_TERMINAL_FAILURE,
            "outer runs exhausted with a rejected resumed selected sample",
        )?;
        Err(cyanos::AgentRuntimeError::new(format!(
            "task {} resumed rejected run {run_index}, but no outer runs remain; runtime checkpoint tag={tag}; next action: inspect verifier feedback and rerun after fixing the task or implementation",
            command.task_id().as_str()
        ))
        .into())
    }

    fn selected_outcome_for_resume(
        task: &cyanos::TaskLayout,
        run_index: usize,
    ) -> Result<SampleRunOutcome, cyanos::AppError> {
        let content = fs::read_to_string(task.result()).map_err(|source| {
            cyanos::AppError::io("read task result for selected resume", source)
        })?;
        let result = cyanos::TaskResult::from_json(&content).map_err(|source| {
            cyanos::AppError::io(
                "parse task result for selected resume",
                io::Error::new(io::ErrorKind::InvalidData, source),
            )
        })?;
        let record = result
            .runs()
            .iter()
            .find(|record| record.run_index() == run_index)
            .ok_or_else(|| {
                cyanos::AgentRuntimeError::new(format!(
                    "task {} resume expected selected run {run_index} in result.json; next action: inspect ledger.jsonl and result.json before rerunning",
                    task.task_id().as_str()
                ))
            })?;
        let sample_id = record.selected_sample();
        let eval_path = Self::resume_artifact_path(task, record.feedback().eval_path());
        let summary_path = task.sample_summary(sample_id, run_index);
        let patch_path = task.sample_patch(sample_id, run_index);
        Self::require_resume_artifact(task, &eval_path, "selected sample eval")?;
        Self::require_resume_artifact(task, &summary_path, "selected sample summary")?;
        Self::require_resume_artifact(task, &patch_path, "selected sample patch")?;
        let eval_content = fs::read_to_string(&eval_path)
            .map_err(|source| cyanos::AppError::io("read selected sample eval", source))?;
        let snapshot = cyanos::EvaluationSnapshot::parse(&eval_content).map_err(|source| {
            cyanos::AppError::io(
                "parse selected sample eval",
                io::Error::new(io::ErrorKind::InvalidData, source),
            )
        })?;

        Ok(SampleRunOutcome {
            run_index,
            sample_id,
            evaluation: Self::evaluation_from_snapshot(&snapshot),
            score: record.selected_score(),
            eval_path,
            summary_path,
            patch_path,
        })
    }

    fn resume_artifact_path(task: &cyanos::TaskLayout, label: &str) -> PathBuf {
        let path = PathBuf::from(label);
        if path.is_absolute() {
            path
        } else {
            task.root().join(path)
        }
    }

    fn require_resume_artifact(
        task: &cyanos::TaskLayout,
        path: &Path,
        label: &str,
    ) -> Result<(), cyanos::AppError> {
        if path.is_file() {
            return Ok(());
        }
        Err(cyanos::AgentRuntimeError::new(format!(
            "task {} resume expected {label} at {}; next action: inspect runtime artifacts before rerunning",
            task.task_id().as_str(),
            path.display()
        ))
        .into())
    }

    fn evaluation_from_snapshot(snapshot: &cyanos::EvaluationSnapshot) -> cyanos::Evaluation {
        let findings = snapshot
            .findings()
            .iter()
            .cloned()
            .map(cyanos::EvaluationFinding::new)
            .collect::<Vec<_>>();
        let evaluation = match snapshot.verdict() {
            cyanos::EvaluationVerdict::Accepted => {
                cyanos::Evaluation::accepted_with_findings(findings)
            }
            cyanos::EvaluationVerdict::Rejected => cyanos::Evaluation::rejected(findings),
        };
        evaluation.with_evidence(
            snapshot.structural_evidence().to_vec(),
            snapshot.recall_evidence().to_vec(),
        )
    }

    fn final_sample_summary(
        invocation: &cyanos::Invocation,
        task: &cyanos::TaskLayout,
        run_index: usize,
        selected: &SampleRunOutcome,
        pr: Option<&str>,
    ) -> String {
        let mut summary = format!(
            "{} dependencies=ok task={} prompt=prompt.{}.md selected_sample={} score={} summary={} patch={} eval={}",
            invocation.render(),
            task.task_id().as_str(),
            run_index.saturating_sub(1),
            selected.sample_id,
            selected.score.total().as_i64(),
            Self::relative_label(task, &selected.summary_path),
            Self::relative_label(task, &selected.patch_path),
            Self::relative_label(task, &selected.eval_path)
        );
        if let Some(pr) = pr {
            summary.push_str(" pr=");
            summary.push_str(pr);
        }
        summary
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "resume handling needs project, task, command, prompt, and mutable output state"
    )]
    fn resume_delivery_pr_state(
        state_lines: &mut Vec<String>,
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
        command: &cyanos::RunCommand,
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        current_prompt: &mut String,
        resume_plan: &cyanos::ResumePlan,
    ) -> Result<Option<String>, cyanos::AppError> {
        if !matches!(
            resume_plan.state,
            cyanos::ResumeState::PrOpened
                | cyanos::ResumeState::PrFeedback
                | cyanos::ResumeState::PrReady
                | cyanos::ResumeState::TerminalFailure
        ) {
            return Ok(None);
        }
        let Some(identity) = resume_plan.delivery_pr.as_ref() else {
            return Ok(None);
        };
        let pr = identity.url.as_str();
        if let Some(feedback) = Self::pr_readiness_feedback(pr, config)? {
            cyanos::RuntimeCheckpoint::append_ledger(task, STATE_PR_FEEDBACK, &feedback)?;
            state_lines.push(format!(
                "cyanos: project={} task={} state=pr_feedback summary={}",
                command.project_id().as_str(),
                command.task_id().as_str(),
                cyanos::RuntimeCheckpoint::sanitize_state_value(&feedback)
            ));
            let tag = Self::publish_runtime_checkpoint(
                project,
                config,
                command.task_id(),
                &format!("pr-feedback-run-{}", resume_plan.last_run_index.max(1)),
            )?;
            state_lines.push(format!(
                "cyanos: project={} task={} state=runtime_pushed tag={tag}",
                command.project_id().as_str(),
                command.task_id().as_str()
            ));
            if resume_plan.next_run_index > OUTER_RUN_LIMIT {
                let tag = Self::publish_runtime_checkpoint(
                    project,
                    config,
                    command.task_id(),
                    "terminal-failure",
                )?;
                cyanos::RuntimeCheckpoint::append_ledger(
                    task,
                    STATE_TERMINAL_FAILURE,
                    "PR readiness feedback remained unresolved during resume",
                )?;
                return Err(cyanos::AgentRuntimeError::new(format!(
                    "task {} resumed from {} but no outer runs remain; PR readiness still has unresolved feedback for {pr}; runtime checkpoint tag={tag}; feedback={}; next action: inspect the PR checks/review comments and rerun after addressing the blocker",
                    command.task_id().as_str(),
                    resume_plan.state.as_str(),
                    cyanos::RuntimeCheckpoint::sanitize_state_value(&feedback)
                ))
                .into());
            }
            let prompt_run_index = resume_plan.last_run_index.max(1);
            if !task.prompt_snapshot(prompt_run_index).is_file() {
                let evolved = Self::evolve_prompt_from_feedback(
                    task,
                    prompts,
                    current_prompt,
                    &feedback,
                    prompt_run_index,
                )?;
                *current_prompt = evolved;
                cyanos::RuntimeCheckpoint::append_ledger(
                    task,
                    STATE_PROMPT_EVOLVED,
                    &format!("wrote prompt.{prompt_run_index}.md from resumed PR feedback"),
                )?;
            }
            return Ok(None);
        }

        let tag = Self::publish_runtime_checkpoint(project, config, command.task_id(), "pr-ready")?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=runtime_pushed tag={tag}",
            command.project_id().as_str(),
            command.task_id().as_str()
        ));
        cyanos::RuntimeCheckpoint::append_ledger(task, STATE_PR_READY, "ready for user merge")?;
        state_lines.push(format!(
            "cyanos: project={} task={} state=pr_ready pr={pr}",
            command.project_id().as_str(),
            command.task_id().as_str()
        ));
        Ok(Some(state_lines.join("\n")))
    }

    fn record_task_result_run(
        task_result: &mut cyanos::TaskResult,
        run_index: usize,
        selected: &SampleRunOutcome,
        task: &cyanos::TaskLayout,
    ) {
        let findings = selected
            .evaluation
            .findings()
            .iter()
            .map(|finding| finding.message().to_owned())
            .collect();
        let feedback = cyanos::VerifierFeedback::new(
            Self::relative_label(task, &selected.eval_path),
            if selected.evaluation.is_accepted() {
                "sample passed repository verifier".to_owned()
            } else {
                "sample failed repository verifier".to_owned()
            },
            if selected.evaluation.is_accepted() {
                "none".to_owned()
            } else {
                VERIFICATION_FAILURE_CLASS.to_owned()
            },
        )
        .with_findings(findings)
        .with_next_action(if selected.evaluation.is_accepted() {
            "promote verified candidate and open or update PR".to_owned()
        } else {
            "revise prompt and rerun samples before promotion".to_owned()
        });
        task_result.record_run(run_index, selected.sample_id, selected.score, feedback);
    }

    fn evolve_prompt(
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        current_prompt: &str,
        selected: &SampleRunOutcome,
        next_prompt_index: usize,
    ) -> Result<String, cyanos::AppError> {
        Self::evolve_prompt_from_evaluation(
            task,
            prompts,
            current_prompt,
            &selected.evaluation,
            next_prompt_index,
        )
    }

    fn evolve_prompt_from_feedback(
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        current_prompt: &str,
        feedback: &str,
        next_prompt_index: usize,
    ) -> Result<String, cyanos::AppError> {
        let evaluation =
            cyanos::Evaluation::rejected(vec![cyanos::EvaluationFinding::new(feedback.to_owned())]);
        Self::evolve_prompt_from_evaluation(
            task,
            prompts,
            current_prompt,
            &evaluation,
            next_prompt_index,
        )
    }

    fn evolve_prompt_from_evaluation(
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        current_prompt: &str,
        evaluation: &cyanos::Evaluation,
        next_prompt_index: usize,
    ) -> Result<String, cyanos::AppError> {
        let prompt_reviser = cyanos::HypothesisPromptReviser::default();
        let current = cyanos::InnerPrompt::new(current_prompt.to_owned());
        let revision =
            cyanos::PromptReviser::revise(&prompt_reviser, prompts.outer(), &current, evaluation);
        let revised_prompt = revision.prompt().as_str().to_owned();

        fs::write(task.root().join(RUNTIME_PROMPT_FILE), &revised_prompt)
            .map_err(|source| cyanos::AppError::io("write evolved prompt", source))?;
        fs::write(task.prompt_snapshot(next_prompt_index), &revised_prompt)
            .map_err(|source| cyanos::AppError::io("write evolved prompt snapshot", source))?;

        Ok(revised_prompt)
    }

    fn run_llm_judge(
        command: &cyanos::RunCommand,
        worktree: &Path,
        prompt: &str,
    ) -> Result<cyanos::JudgeReview, cyanos::AppError> {
        let judge_dir = std::env::temp_dir().join(format!(
            "cyanos-judge-{}-{}",
            std::process::id(),
            monotonic_nanos()
        ));
        fs::create_dir_all(&judge_dir)
            .map_err(|source| cyanos::AppError::io("create judge worktree", source))?;
        let judge_identity = cyanos::JudgeIdentity::for_worker(command.agent());
        let mut runtime = cyanos::SystemAgentRuntime::new_with_execution_mode(
            judge_identity.agent(),
            judge_identity.model().clone(),
            cyanos::AgentExecutionMode::JudgeIsolated,
            judge_dir.clone(),
            cyanos::SystemAgentCommandRunner,
        );
        let turn = cyanos::AgentTurn::new(
            cyanos::JUDGE_SYSTEM_PROMPT.to_owned(),
            format!(
                "{prompt}\n\nOriginal candidate worktree path for reference only: {}",
                worktree.display()
            ),
        );
        let mut observer = cyanos::NoopAgentStreamObserver;
        let result =
            Self::execute_agent_with_retry(&mut runtime, &turn, &mut observer).map(|output| {
                cyanos::parse_judge_review(output.content()).with_identity(&judge_identity)
            });
        let cleanup = fs::remove_dir_all(&judge_dir)
            .map_err(|source| cyanos::AppError::io("remove judge worktree", source));
        match (result, cleanup) {
            (Ok(review), Ok(())) => Ok(review),
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        }
    }

    fn measure_coverage(worktree: &Path) -> Result<i64, cyanos::AppError> {
        if std::env::var_os(LLVM_PROFILE_FILE_ENV).is_some()
            && std::env::var_os(FORCE_COVERAGE_VERIFIER_ENV).is_none()
        {
            return Ok(PERFECT_SAMPLE_SCORE);
        }

        let output = Self::command_output(
            CARGO_PROGRAM,
            worktree,
            CARGO_COVERAGE_ARGS,
            "run sample coverage verifier",
        )?;
        if !output.status.success() {
            return Ok(0);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(cyanos::parse_coverage_basis_points(&stdout).unwrap_or(0))
    }

    fn write_sample_patch(worktree: &Path, patch_path: &Path) -> Result<(), cyanos::AppError> {
        let add_output = Self::command_output(
            GIT_PROGRAM,
            worktree,
            &[
                "add",
                "-N",
                "--",
                GIT_PATHSPEC_ALL,
                GIT_PATHSPEC_EXCLUDE_TARGET,
                GIT_PATHSPEC_EXCLUDE_PROFRAW,
            ],
            "prepare sample patch",
        )?;
        if !add_output.status.success() {
            return Err(cyanos::AgentRuntimeError::new(format!(
                "git add -N failed in {} with exit code {:?}",
                worktree.display(),
                add_output.status.code()
            ))
            .into());
        }
        let patch = Self::command_output(
            GIT_PROGRAM,
            worktree,
            &[
                "diff",
                "--binary",
                "HEAD",
                "--",
                GIT_PATHSPEC_ALL,
                GIT_PATHSPEC_EXCLUDE_TARGET,
                GIT_PATHSPEC_EXCLUDE_PROFRAW,
            ],
            "write sample patch",
        )?;
        if !patch.status.success() {
            return Err(cyanos::AgentRuntimeError::new(format!(
                "git diff failed in {} with exit code {:?}",
                worktree.display(),
                patch.status.code()
            ))
            .into());
        }
        fs::write(patch_path, patch.stdout)
            .map_err(|source| cyanos::AppError::io("write sample patch", source))
    }

    fn relative_label(task: &cyanos::TaskLayout, path: &std::path::Path) -> String {
        path.strip_prefix(task.root()).map_or_else(
            |_| path.to_string_lossy().to_string(),
            |path| path.to_string_lossy().to_string(),
        )
    }

    fn load_required_project_config(
        project: &cyanos::ProjectLayout,
    ) -> Result<cyanos::ProjectConfig, cyanos::AppError> {
        if !project.config().is_file() {
            return Err(cyanos::TaskRunError::MissingProjectConfig(project.config()).into());
        }

        let config = cyanos::ProjectConfig::load(project)?;
        if config.repo() == UNKNOWN_REPOSITORY || !config.repo().contains('/') {
            return Err(cyanos::TaskRunError::UnknownRepository(project.config()).into());
        }

        Ok(config)
    }

    fn preflight_runtime_repository(
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
    ) -> Result<(), cyanos::AppError> {
        if config.runtime_repo() == config.repo() || !config.runtime_repo().contains('/') {
            return Err(cyanos::TaskRunError::RuntimeRepositoryInvalid {
                source_repo: config.repo().to_owned(),
                runtime_repo: config.runtime_repo().to_owned(),
            }
            .into());
        }

        if !Self::github_repo_exists(config.runtime_repo())? {
            Self::create_runtime_github_repo(config.runtime_repo())?;
        }

        Self::require_repository_write_permission(
            GH_PROGRAM,
            config.runtime_repo(),
            "runtime repository",
        )?;
        Self::ensure_runtime_git_repository(project, config)
    }

    fn preflight_target_repository(config: &cyanos::ProjectConfig) -> Result<(), cyanos::AppError> {
        Self::require_repository_write_permission(GH_PROGRAM, config.repo(), "target repository")
    }

    fn github_repo_exists(runtime_repo: &str) -> Result<bool, cyanos::AppError> {
        Self::github_repo_exists_with_program(GH_PROGRAM, runtime_repo)
    }

    fn github_repo_exists_with_program(
        program: &str,
        runtime_repo: &str,
    ) -> Result<bool, cyanos::AppError> {
        let output = Self::command_output_retrying_errors(
            program,
            Path::new("."),
            &["repo", "view", runtime_repo, "--json", "nameWithOwner"],
            "check runtime repository",
        )?;
        Ok(output.status.success())
    }

    fn create_runtime_github_repo(runtime_repo: &str) -> Result<(), cyanos::AppError> {
        Self::create_runtime_github_repo_with_program(GH_PROGRAM, runtime_repo)
    }

    fn create_runtime_github_repo_with_program(
        program: &str,
        runtime_repo: &str,
    ) -> Result<(), cyanos::AppError> {
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &["repo", "create", runtime_repo, "--private"],
                "create runtime repository",
            )?;
            if output.status.success() {
                return Ok(());
            }
            last_code = output.status.code();
        }

        Err(cyanos::TaskRunError::RuntimeRepositoryFailed {
            runtime_repo: runtime_repo.to_owned(),
            action: "create",
            code: last_code,
        }
        .into())
    }

    fn require_repository_write_permission(
        program: &str,
        repo: &str,
        label: &str,
    ) -> Result<(), cyanos::AppError> {
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &[
                    "repo",
                    "view",
                    repo,
                    "--json",
                    "viewerPermission",
                    "--jq",
                    ".viewerPermission",
                ],
                "check repository permissions",
            )?;
            if output.status.success() {
                let permission = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if repository_permission_allows_delivery(permission.as_str()) {
                    return Ok(());
                }
                return Err(cyanos::TaskRunError::PreflightFailed {
                    check: format!("{label} {repo} write_permission"),
                    next_action: format!(
                        "grant write permission; current viewerPermission is {permission}, but Cyanos needs comment, branch push, and PR update access"
                    ),
                }
                .into());
            }
            last_code = output.status.code();
            thread::sleep(RETRY_DELAY);
        }

        Err(cyanos::TaskRunError::PreflightFailed {
            check: format!("{label} {repo} reachable"),
            next_action: format!(
                "check repository access and gh authentication before retrying; gh exited with {last_code:?}"
            ),
        }
        .into())
    }

    fn ensure_runtime_git_repository(
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
    ) -> Result<(), cyanos::AppError> {
        let root = project.root();
        if !root.join(GIT_DIR).is_dir() {
            Self::run_runtime_git(
                project,
                config.runtime_repo(),
                &[GIT_INIT_ARG],
                "initialize runtime repository",
            )?;
            Self::run_runtime_git(
                project,
                config.runtime_repo(),
                &["checkout", "-B", RUNTIME_BRANCH],
                "checkout runtime branch",
            )?;
            Self::run_runtime_git(
                project,
                config.runtime_repo(),
                &["config", "user.name", "Cyanos"],
                "configure runtime git user",
            )?;
            Self::run_runtime_git(
                project,
                config.runtime_repo(),
                &["config", "user.email", RUNTIME_GIT_EMAIL],
                "configure runtime git email",
            )?;
        }
        fs::write(root.join(GITIGNORE_FILE), RUNTIME_GITIGNORE)
            .map_err(|source| cyanos::AppError::io("write runtime gitignore", source))?;

        if !Self::has_git_remote(&root) {
            let remote = repository_https_remote(config.runtime_repo());
            Self::run_runtime_git(
                project,
                config.runtime_repo(),
                &["remote", "add", GIT_ORIGIN_REMOTE, remote.as_str()],
                "configure runtime repository remote",
            )?;
        }

        Ok(())
    }

    fn publish_runtime_checkpoint(
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
        task_id: &cyanos::TaskId,
        checkpoint: &str,
    ) -> Result<String, cyanos::AppError> {
        let tag = format!("task-{}-{checkpoint}", task_id.as_str());
        Self::run_runtime_git(
            project,
            config.runtime_repo(),
            &["add", "-A", "--", "."],
            "stage runtime checkpoint",
        )?;
        Self::run_runtime_git(
            project,
            config.runtime_repo(),
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("Checkpoint Cyanos task {} {checkpoint}", task_id.as_str()),
            ],
            "commit runtime checkpoint",
        )?;
        Self::run_runtime_git(
            project,
            config.runtime_repo(),
            &["tag", "-f", tag.as_str()],
            "tag runtime checkpoint",
        )?;

        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let output = Self::git_output_retrying_errors(
                &project.root(),
                &[
                    "push",
                    "--force",
                    GIT_ORIGIN_REMOTE,
                    &format!("HEAD:{RUNTIME_BRANCH}"),
                    &format!("refs/tags/{tag}:refs/tags/{tag}"),
                ],
                "push runtime checkpoint",
            )?;
            if output.status.success() {
                return Ok(tag);
            }
            last_code = output.status.code();
        }

        let task = project.task(task_id.clone());
        let failure_summary = format!(
            "checkpoint={checkpoint} remote={runtime_repo} action=push code={last_code:?}; next_action=check runtime repository permissions and rerun",
            runtime_repo = config.runtime_repo()
        );
        cyanos::RuntimeCheckpoint::append_ledger(
            &task,
            STATE_RUNTIME_PUBLISH_FAILED,
            &failure_summary,
        )?;
        cyanos::RuntimeCheckpoint::append_ledger(&task, STATE_TERMINAL_FAILURE, &failure_summary)?;
        Err(cyanos::TaskRunError::RuntimeRepositoryFailed {
            runtime_repo: config.runtime_repo().to_owned(),
            action: "push",
            code: last_code,
        }
        .into())
    }

    fn run_runtime_git(
        project: &cyanos::ProjectLayout,
        runtime_repo: &str,
        args: &[&str],
        action: &'static str,
    ) -> Result<(), cyanos::AppError> {
        let output = Self::git_output(&project.root(), args, action)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(cyanos::TaskRunError::RuntimeRepositoryFailed {
                runtime_repo: runtime_repo.to_owned(),
                action,
                code: output.status.code(),
            }
            .into())
        }
    }

    fn load_github_task(
        config: &cyanos::ProjectConfig,
        task_id: &cyanos::TaskId,
    ) -> Result<TaskBrief, cyanos::AppError> {
        Self::load_github_task_with_program(GH_PROGRAM, config, task_id)
    }

    fn load_github_task_with_program(
        program: &str,
        config: &cyanos::ProjectConfig,
        task_id: &cyanos::TaskId,
    ) -> Result<TaskBrief, cyanos::AppError> {
        Self::ensure_github_task_is_issue_with_program(program, config, task_id)?;
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &[
                    "issue",
                    "view",
                    task_id.as_str(),
                    "--repo",
                    config.repo(),
                    "--json",
                    "title,body,url",
                    "--template",
                    "{{.url}}\n---CYANOS-TITLE---\n{{.title}}\n---CYANOS-BODY---\n{{.body}}\n",
                ],
                "load GitHub task",
            )?;

            if output.status.success() {
                return TaskBrief::parse(config.repo(), task_id, &output.stdout).ok_or_else(|| {
                    cyanos::AppError::from(cyanos::TaskRunError::GitHubTaskFailed {
                        repo: config.repo().to_owned(),
                        task_id: task_id.as_str().to_owned(),
                        code: output.status.code(),
                    })
                });
            }

            last_code = output.status.code();
        }

        Err(cyanos::TaskRunError::GitHubTaskFailed {
            repo: config.repo().to_owned(),
            task_id: task_id.as_str().to_owned(),
            code: last_code,
        }
        .into())
    }

    fn ensure_github_task_is_issue_with_program(
        program: &str,
        config: &cyanos::ProjectConfig,
        task_id: &cyanos::TaskId,
    ) -> Result<(), cyanos::AppError> {
        let Some((host, owner, repo)) = repo_graphql_target(config.repo()) else {
            return Err(cyanos::AgentRuntimeError::new(format!(
                "cannot verify GitHub task type for repository {}; next action: configure project with an owner/repo locator and retry",
                config.repo()
            ))
            .into());
        };
        let owner_arg = format!("owner={owner}");
        let repo_arg = format!("repo={repo}");
        let number_arg = format!("number={}", task_id.as_str());
        let query_arg = format!("query={GITHUB_TASK_TYPE_QUERY}");
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let mut args = vec!["api"];
            if let Some(host) = host.as_deref() {
                args.extend(["--hostname", host]);
            }
            args.extend([
                "graphql",
                "-f",
                owner_arg.as_str(),
                "-f",
                repo_arg.as_str(),
                "-F",
                number_arg.as_str(),
                "-f",
                query_arg.as_str(),
                "--jq",
                GITHUB_TASK_TYPE_JQ,
            ]);
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &args,
                "verify GitHub task type",
            )?;
            if output.status.success() {
                let object_type = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                return match object_type.as_str() {
                    GITHUB_OBJECT_ISSUE => Ok(()),
                    GITHUB_OBJECT_PULL_REQUEST => Err(cyanos::AgentRuntimeError::new(format!(
                        "task {} in {} resolves to a pull request; Cyanos requires a GitHub issue task brief unless PR task sources are explicitly configured; next action: rerun with the linked issue id or configure an explicit task source mapping",
                        task_id.as_str(),
                        config.repo()
                    ))
                    .into()),
                    _ => Err(cyanos::TaskRunError::GitHubTaskFailed {
                        repo: config.repo().to_owned(),
                        task_id: task_id.as_str().to_owned(),
                        code: output.status.code(),
                    }
                    .into()),
                };
            }
            last_code = output.status.code();
        }

        Err(cyanos::TaskRunError::GitHubTaskFailed {
            repo: config.repo().to_owned(),
            task_id: task_id.as_str().to_owned(),
            code: last_code,
        }
        .into())
    }

    fn post_blocker_comment(
        command: &cyanos::RunCommand,
        config: &cyanos::ProjectConfig,
        brief: &TaskBrief,
        findings: &[TaskBriefFinding],
    ) -> Result<String, cyanos::AppError> {
        Self::post_blocker_comment_with_program(GH_PROGRAM, command, config, brief, findings)
    }

    fn post_blocker_comment_with_program(
        program: &str,
        command: &cyanos::RunCommand,
        config: &cyanos::ProjectConfig,
        brief: &TaskBrief,
        findings: &[TaskBriefFinding],
    ) -> Result<String, cyanos::AppError> {
        let body = blocker_comment_body(command, findings);
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &[
                    "issue",
                    "comment",
                    command.task_id().as_str(),
                    "--repo",
                    config.repo(),
                    "--body",
                    body.as_str(),
                ],
                "post GitHub blocker comment",
            )?;

            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                return Ok(if stdout.is_empty() {
                    brief.url().to_owned()
                } else {
                    stdout
                });
            }

            last_code = output.status.code();
        }

        Err(cyanos::TaskRunError::GitHubCommentFailed {
            repo: config.repo().to_owned(),
            task_id: command.task_id().as_str().to_owned(),
            code: last_code,
        }
        .into())
    }

    fn promote_selected_sample(
        task: &cyanos::TaskLayout,
        selected: &SampleRunOutcome,
    ) -> Result<(), cyanos::AppError> {
        Self::ensure_promotion_worktree(task)?;
        Self::apply_patch_to_worktree(task, &selected.patch_path)?;
        Self::commit_if_changed(
            &task.worktree(),
            &format!("Promote Cyanos task {}", task.task_id().as_str()),
        )?;
        Self::run_git(
            &task.worktree(),
            &["branch", "-f", task.pr_branch_name().as_str(), "HEAD"],
            "prepare feature branch",
        )?;
        Self::write_best_metadata(task, selected)
    }

    fn ensure_promotion_worktree(task: &cyanos::TaskLayout) -> Result<(), cyanos::AppError> {
        if task.worktree().is_dir() {
            return Ok(());
        }

        let base_ref = Self::promotion_base_ref(task)?;
        let branch = task.evolution_branch_name();
        let worktree = task.worktree().to_string_lossy().to_string();
        Self::run_git(
            &task.project().origin(),
            &[
                GIT_WORKTREE_ARG,
                "add",
                "-B",
                branch.as_str(),
                worktree.as_str(),
                base_ref.as_str(),
            ],
            "materialize promoted task checkout",
        )
    }

    fn promotion_base_ref(task: &cyanos::TaskLayout) -> Result<String, cyanos::AppError> {
        let origin = task.project().origin();
        let evolution_branch = task.evolution_branch_name();
        if Self::git_ref_exists(&origin, &evolution_branch)? {
            return Ok(evolution_branch);
        }
        if Self::git_ref_exists(&origin, GIT_ORIGIN_HEAD_REF)? {
            return Ok(GIT_ORIGIN_HEAD_REF.to_owned());
        }
        Ok("HEAD".to_owned())
    }

    fn git_ref_exists(worktree: &Path, reference: &str) -> Result<bool, cyanos::AppError> {
        let output = Self::git_output(
            worktree,
            &["rev-parse", "--verify", reference],
            "check git ref",
        )?;
        Ok(output.status.success())
    }

    fn write_best_metadata(
        task: &cyanos::TaskLayout,
        selected: &SampleRunOutcome,
    ) -> Result<(), cyanos::AppError> {
        let head = Self::current_head(&task.worktree())?;
        let record = cyanos::BestRecord::new(
            task.task_id().as_str(),
            selected.run_index,
            selected.sample_id,
            selected.score,
            Self::relative_label(task, &selected.eval_path),
            Self::relative_label(task, &selected.patch_path),
            head.as_str(),
            head.as_str(),
            head.as_str(),
        );
        cyanos::write_best_record(&task.best(), &record)
            .map_err(|source| cyanos::AppError::io("write global best metadata", source))
    }

    fn push_and_open_pr(
        task: &cyanos::TaskLayout,
        selected: &SampleRunOutcome,
        config: &cyanos::ProjectConfig,
        task_readme: &str,
    ) -> Result<String, cyanos::AppError> {
        if config.repo() == UNKNOWN_REPOSITORY {
            return Err(cyanos::TaskRunError::UnknownRepository(
                config.source_path().to_path_buf(),
            )
            .into());
        }
        if !Self::has_git_remote(&task.worktree()) {
            return Err(cyanos::AgentRuntimeError::new(format!(
                "cannot open PR for task {} because promoted worktree {} has no origin remote; next action: configure the target repository remote and rerun Cyanos",
                task.task_id().as_str(),
                task.worktree().display()
            ))
            .into());
        }

        let default_delivery_branch = task.pr_branch_name();
        let delivery_identity = Self::delivery_pr_for_push(task, config)?;
        let delivery_branch = delivery_identity
            .as_ref()
            .map_or(default_delivery_branch.as_str(), |identity| {
                identity.head.as_str()
            });
        if delivery_branch != default_delivery_branch {
            Self::run_git(
                &task.worktree(),
                &["branch", "-f", delivery_branch, "HEAD"],
                "prepare adopted delivery branch",
            )?;
        }
        let evolution_branch = task.evolution_branch_name();
        Self::run_git(
            &task.worktree(),
            &[
                "push",
                "-f",
                GIT_ORIGIN_REMOTE,
                evolution_branch.as_str(),
                delivery_branch,
            ],
            "push promoted branches",
        )?;
        Self::open_or_update_pr(task, selected, config, task_readme)
    }

    fn apply_patch_to_worktree(
        task: &cyanos::TaskLayout,
        patch_path: &Path,
    ) -> Result<(), cyanos::AppError> {
        let patch = fs::read(patch_path)
            .map_err(|source| cyanos::AppError::io("read selected sample patch", source))?;
        if patch.is_empty() {
            return Ok(());
        }
        let already_applied = Self::command_output(
            GIT_PROGRAM,
            &task.worktree(),
            &[
                "apply",
                "--reverse",
                "--check",
                patch_path.to_string_lossy().as_ref(),
            ],
            "check selected sample patch already applied",
        )?;
        if already_applied.status.success() {
            return Ok(());
        }
        let output = Self::command_output(
            GIT_PROGRAM,
            &task.worktree(),
            &["apply", "--index", patch_path.to_string_lossy().as_ref()],
            "apply selected sample patch",
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(cyanos::AgentRuntimeError::new(format!(
                "git apply failed in {} with exit code {:?}",
                task.worktree().display(),
                output.status.code()
            ))
            .into())
        }
    }

    fn commit_if_changed(worktree: &Path, message: &str) -> Result<(), cyanos::AppError> {
        let status = Self::git_output(
            worktree,
            &[
                GIT_STATUS_ARG,
                GIT_STATUS_PORCELAIN_ARG,
                GIT_STATUS_UNTRACKED_ARG,
            ],
            "check promoted worktree status",
        )?;
        if status.stdout.is_empty() {
            return Ok(());
        }
        Self::run_git(worktree, &["add", "-A"], "stage promoted worktree")?;
        Self::run_git(
            worktree,
            &["commit", "-m", message],
            "commit promoted worktree",
        )
    }

    fn open_or_update_pr(
        task: &cyanos::TaskLayout,
        selected: &SampleRunOutcome,
        config: &cyanos::ProjectConfig,
        task_readme: &str,
    ) -> Result<String, cyanos::AppError> {
        Self::open_or_update_pr_with_program(GH_PROGRAM, task, selected, config, task_readme)
    }

    fn open_or_update_pr_with_program(
        program: &str,
        task: &cyanos::TaskLayout,
        selected: &SampleRunOutcome,
        config: &cyanos::ProjectConfig,
        task_readme: &str,
    ) -> Result<String, cyanos::AppError> {
        let head = task.pr_branch_name();
        let body = Self::pr_body(task, selected, task_readme);
        let title = format!("Cyanos task {}", task.task_id().as_str());
        if let Some(identity) = cyanos::read_delivery_pr_identity(task)? {
            return Self::edit_pr_with_program(
                program,
                identity.url.as_str(),
                config,
                &title,
                &body,
            );
        }
        let existing = Self::command_output_retrying_errors(
            program,
            Path::new("."),
            &[
                "pr",
                "view",
                head.as_str(),
                "--repo",
                config.repo(),
                "--json",
                "url",
                "--jq",
                ".url",
            ],
            "view pull request",
        )?;
        if existing.status.success() && !existing.stdout.is_empty() {
            let pr_url = String::from_utf8_lossy(&existing.stdout).trim().to_owned();
            cyanos::write_delivery_pr_identity(
                task,
                &cyanos::DeliveryPrIdentity::new(&pr_url, &head),
            )?;
            return Self::edit_pr_with_program(program, pr_url.as_str(), config, &title, &body);
        }

        if let Some(identity) = Self::find_adoptable_pr_by_task_with_program(program, task, config)?
        {
            cyanos::write_delivery_pr_identity(task, &identity)?;
            return Self::edit_pr_with_program(
                program,
                identity.url.as_str(),
                config,
                &title,
                &body,
            );
        }

        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let created = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &[
                    "pr",
                    "create",
                    "--repo",
                    config.repo(),
                    "--head",
                    head.as_str(),
                    "--base",
                    config.base_branch(),
                    "--title",
                    title.as_str(),
                    "--body",
                    body.as_str(),
                ],
                "create pull request",
            )?;
            if created.status.success() {
                let pr_url = String::from_utf8_lossy(&created.stdout).trim().to_owned();
                cyanos::write_delivery_pr_identity(
                    task,
                    &cyanos::DeliveryPrIdentity::new(&pr_url, &head),
                )?;
                return Ok(pr_url);
            }
            last_code = created.status.code();
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh pr create failed for {} with exit code {:?}",
            config.repo(),
            last_code
        ))
        .into())
    }

    fn edit_pr_with_program(
        program: &str,
        pr_url: &str,
        config: &cyanos::ProjectConfig,
        title: &str,
        body: &str,
    ) -> Result<String, cyanos::AppError> {
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let edited = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &[
                    "pr",
                    "edit",
                    pr_url,
                    "--repo",
                    config.repo(),
                    "--title",
                    title,
                    "--body",
                    body,
                ],
                "update pull request",
            )?;
            if edited.status.success() {
                return Ok(pr_url.to_owned());
            }
            last_code = edited.status.code();
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh pr edit failed for {} with exit code {:?}",
            config.repo(),
            last_code
        ))
        .into())
    }

    fn delivery_pr_for_push(
        task: &cyanos::TaskLayout,
        config: &cyanos::ProjectConfig,
    ) -> Result<Option<cyanos::DeliveryPrIdentity>, cyanos::AppError> {
        if let Some(identity) = cyanos::read_delivery_pr_identity(task)? {
            return Ok(Some(identity));
        }
        let identity = Self::find_adoptable_pr_by_task_with_program(GH_PROGRAM, task, config)?;
        if let Some(identity) = identity.as_ref() {
            cyanos::write_delivery_pr_identity(task, identity)?;
        }
        Ok(identity)
    }

    fn find_adoptable_pr_by_task_with_program(
        program: &str,
        task: &cyanos::TaskLayout,
        config: &cyanos::ProjectConfig,
    ) -> Result<Option<cyanos::DeliveryPrIdentity>, cyanos::AppError> {
        let jq = r#".[] | [.url, (.headRefName // ""), (.title // ""), (.body // "")] | @tsv"#;
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &[
                    "pr",
                    "list",
                    "--repo",
                    config.repo(),
                    "--state",
                    "open",
                    "--json",
                    PR_LIST_JSON_FIELDS,
                    "--jq",
                    jq,
                ],
                "find adoptable pull request",
            )?;
            if output.status.success() {
                let candidates = cyanos::delivery_pr_candidates_from_tsv(
                    &output.stdout,
                    task.pr_branch_name().as_str(),
                    task.task_id().as_str(),
                );
                return match candidates.as_slice() {
                    [] => Ok(None),
                    [identity] => Ok(Some(identity.clone())),
                    _ => Err(cyanos::AgentRuntimeError::new(format!(
                        "ambiguous existing delivery PRs for task {} in {}; next action: persist {} with the intended PR URL/head or close unrelated PRs before rerunning",
                        task.task_id().as_str(),
                        config.repo(),
                        cyanos::delivery_pr_identity_path(task).display()
                    ))
                    .into()),
                };
            }
            last_code = output.status.code();
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh pr list failed for {} while checking existing delivery PRs with exit code {:?}; next action: check gh authentication and rerun before Cyanos creates a PR",
            config.repo(),
            last_code
        ))
        .into())
    }

    fn pr_readiness_feedback(
        pr: &str,
        config: &cyanos::ProjectConfig,
    ) -> Result<Option<String>, cyanos::AppError> {
        Self::pr_readiness_feedback_with_program(GH_PROGRAM, pr, config)
    }

    fn pr_readiness_feedback_with_program(
        program: &str,
        pr: &str,
        config: &cyanos::ProjectConfig,
    ) -> Result<Option<String>, cyanos::AppError> {
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &[
                    "pr",
                    "view",
                    pr,
                    "--repo",
                    config.repo(),
                    "--json",
                    PR_READINESS_JSON_FIELDS,
                    "--template",
                    "{{.reviewDecision}}\n{{.mergeStateStatus}}\n{{range .statusCheckRollup}}{{.name}}\t{{.conclusion}}\t{{.status}}\t{{.detailsUrl}}\n{{end}}",
                ],
                "check pull request readiness",
            )?;
            if output.status.success() {
                let report_feedback =
                    pr_readiness_feedback_from_report(&String::from_utf8_lossy(&output.stdout));
                if report_feedback.is_some() {
                    return Ok(report_feedback);
                }
                return Self::pr_unresolved_review_feedback_with_program(program, pr, config);
            }
            last_code = output.status.code();
            thread::sleep(RETRY_DELAY);
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh pr view failed for {pr} in {} after {} attempt(s) with exit code {:?}; next action: check gh authentication, network access, and PR permissions before retrying",
            config.repo(),
            RETRY_ATTEMPTS,
            last_code
        ))
        .into())
    }

    fn pr_unresolved_review_feedback_with_program(
        program: &str,
        pr: &str,
        config: &cyanos::ProjectConfig,
    ) -> Result<Option<String>, cyanos::AppError> {
        let Some((host, owner, repo)) = repo_graphql_target(config.repo()) else {
            return Err(cyanos::AgentRuntimeError::new(format!(
                "cannot check PR review threads for repository {}; next action: configure project with an owner/repo locator and retry",
                config.repo()
            ))
            .into());
        };
        let Some(number) = pr_number_from_url(pr) else {
            return Err(cyanos::AgentRuntimeError::new(format!(
                "cannot check PR review threads for {pr}; next action: rerun after Cyanos opens a GitHub PR URL"
            ))
            .into());
        };
        let owner_arg = format!("owner={owner}");
        let repo_arg = format!("repo={repo}");
        let number_arg = format!("number={number}");
        let query_arg = format!("query={PR_REVIEW_THREADS_QUERY}");
        let target = PullRequestReviewTarget {
            host: host.as_deref(),
            owner: &owner,
            repo: &repo,
            number: &number,
            pr,
        };
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let mut args = vec!["api"];
            if let Some(host) = target.host {
                args.extend(["--hostname", host]);
            }
            args.extend([
                "graphql",
                "-f",
                owner_arg.as_str(),
                "-f",
                repo_arg.as_str(),
                "-F",
                number_arg.as_str(),
                "-f",
                query_arg.as_str(),
                "--jq",
                PR_REVIEW_THREADS_TSV_JQ,
            ]);
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &args,
                "check pull request review threads",
            )?;
            if output.status.success() {
                return Self::pr_review_feedback_from_thread_output(
                    program,
                    config,
                    target,
                    &output.stdout,
                );
            }
            last_code = output.status.code();
            thread::sleep(RETRY_DELAY);
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh api graphql failed for {pr} in {} after {} attempt(s) with exit code {:?}; next action: check gh authentication, network access, and PR review permissions before retrying",
            config.repo(),
            RETRY_ATTEMPTS,
            last_code
        ))
        .into())
    }

    fn pr_review_feedback_from_thread_output(
        program: &str,
        config: &cyanos::ProjectConfig,
        target: PullRequestReviewTarget<'_>,
        output: &[u8],
    ) -> Result<Option<String>, cyanos::AppError> {
        let records = cyanos::review_comment_records_from_tsv(output)?;
        let managed_actor = Self::github_actor_with_program(program, target.host)?;
        let current_diff = Self::pr_diff_with_program(program, target.pr, config)?;
        let resolution_evidence =
            Self::pr_resolution_evidence_with_program(program, target.pr, config)?;
        let evidence = cyanos::ReviewThreadEvidence::new(&current_diff)
            .with_managed_comment_author(&managed_actor)
            .with_fixed_thread_ids(&resolution_evidence.fixed_thread_ids);
        for thread_id in cyanos::unresolved_outdated_thread_ids(&records) {
            Self::resolve_review_thread_with_program(program, target.host, &thread_id)?;
        }
        let fixed_thread_ids =
            cyanos::unresolved_fixed_thread_ids_with_evidence(&records, evidence);
        if !fixed_thread_ids.is_empty() {
            Self::reply_and_resolve_fixed_review_threads(
                program,
                target,
                FixedReviewThreadResolution {
                    records: &records,
                    managed_actor: &managed_actor,
                    thread_ids: &fixed_thread_ids,
                    evidence: &resolution_evidence,
                },
            )?;
        }
        Ok(cyanos::review_feedback_from_comment_records_with_evidence(
            &records, evidence,
        ))
    }

    fn reply_and_resolve_fixed_review_threads(
        program: &str,
        target: PullRequestReviewTarget<'_>,
        resolution: FixedReviewThreadResolution<'_>,
    ) -> Result<(), cyanos::AppError> {
        for thread_id in resolution.thread_ids {
            if !cyanos::review_thread_has_managed_fixed_resolution(
                resolution.records,
                thread_id,
                resolution.managed_actor,
            ) {
                let Some(comment_id) =
                    Self::review_thread_reply_comment_id(resolution.records, thread_id)
                else {
                    return Err(cyanos::AgentRuntimeError::new(format!(
                        "cannot reply to fixed review thread {thread_id}; next action: rerun with a current gh CLI that returns review comment database ids"
                    ))
                    .into());
                };
                Self::reply_to_review_comment_with_program(
                    program,
                    target,
                    comment_id,
                    resolution.evidence,
                )?;
            }
            Self::resolve_review_thread_with_program(program, target.host, thread_id)?;
        }
        Ok(())
    }

    fn github_actor_with_program(
        program: &str,
        host: Option<&str>,
    ) -> Result<String, cyanos::AppError> {
        let query_arg = format!("query={VIEWER_LOGIN_QUERY}");
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let mut args = vec!["api"];
            if let Some(host) = host {
                args.extend(["--hostname", host]);
            }
            args.extend(["graphql", "-f", query_arg.as_str(), "--jq", VIEWER_LOGIN_JQ]);
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &args,
                "detect authenticated GitHub actor",
            )?;
            if output.status.success() {
                let actor = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if !actor.is_empty() {
                    return Ok(actor);
                }
            }
            last_code = output.status.code();
            thread::sleep(RETRY_DELAY);
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh api graphql failed to detect authenticated GitHub actor after {RETRY_ATTEMPTS} attempt(s) with exit code {last_code:?}; next action: check gh authentication before retrying PR readiness"
        ))
        .into())
    }

    fn pr_diff_with_program(
        program: &str,
        pr: &str,
        config: &cyanos::ProjectConfig,
    ) -> Result<String, cyanos::AppError> {
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &["pr", "diff", pr, "--repo", config.repo(), "--patch"],
                "fetch pull request diff",
            )?;
            if output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
            }
            last_code = output.status.code();
            thread::sleep(RETRY_DELAY);
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh pr diff failed for {pr} in {} after {} attempt(s) with exit code {:?}; next action: check gh authentication, network access, and PR diff permissions before retrying",
            config.repo(),
            RETRY_ATTEMPTS,
            last_code
        ))
        .into())
    }

    fn pr_resolution_evidence_with_program(
        program: &str,
        pr: &str,
        config: &cyanos::ProjectConfig,
    ) -> Result<ReviewResolutionEvidence, cyanos::AppError> {
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &[
                    "pr",
                    "view",
                    pr,
                    "--repo",
                    config.repo(),
                    "--json",
                    "headRefOid,body",
                    "--template",
                    "{{.headRefOid}}\n{{.body}}",
                ],
                "fetch pull request resolution evidence",
            )?;
            if output.status.success() {
                let mut evidence = Self::parse_pr_resolution_evidence(&output.stdout)?;
                let body = Self::pr_body_from_resolution_output(&output.stdout);
                if let Some(runtime_evidence) = Self::fixed_review_thread_ids_from_runtime_evidence(
                    config,
                    &body,
                    &evidence.commit,
                )? {
                    evidence.eval = runtime_evidence.eval;
                    evidence.fixed_thread_ids = runtime_evidence.fixed_thread_ids;
                }
                return Ok(evidence);
            }
            last_code = output.status.code();
            thread::sleep(RETRY_DELAY);
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh pr view failed to fetch resolution evidence for {pr} in {} after {} attempt(s) with exit code {:?}; next action: check gh permissions and retry PR readiness",
            config.repo(),
            RETRY_ATTEMPTS,
            last_code
        ))
        .into())
    }

    fn parse_pr_resolution_evidence(
        output: &[u8],
    ) -> Result<ReviewResolutionEvidence, cyanos::AppError> {
        let rendered = String::from_utf8_lossy(output);
        let mut lines = rendered.lines();
        let commit = lines.next().unwrap_or_default().trim().to_owned();
        if commit.is_empty() {
            return Err(cyanos::AgentRuntimeError::new(
                "missing pull request head commit for review-thread resolution; next action: rerun with a current gh CLI"
                    .to_owned(),
            )
            .into());
        }
        let body = lines.collect::<Vec<_>>().join("\n");
        let eval = Self::pr_evaluation_path_from_body(&body)
            .unwrap_or_else(|| "pr-readiness-review-evidence".to_owned());
        Ok(ReviewResolutionEvidence {
            commit,
            eval,
            fixed_thread_ids: Vec::new(),
        })
    }

    fn pr_body_from_resolution_output(output: &[u8]) -> String {
        String::from_utf8_lossy(output)
            .lines()
            .skip(1)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn pr_evaluation_path_from_body(body: &str) -> Option<String> {
        body.lines().find_map(|line| {
            line.trim()
                .strip_prefix("- Evaluation:")
                .map(|value| value.trim().trim_matches('`').to_owned())
                .filter(|value| !value.is_empty())
        })
    }

    fn fixed_review_thread_ids_from_runtime_evidence(
        config: &cyanos::ProjectConfig,
        body: &str,
        head_commit: &str,
    ) -> Result<Option<RuntimeReviewThreadEvidence>, cyanos::AppError> {
        let Some(task) = Self::pr_task_layout(config, body) else {
            return Ok(None);
        };
        let best = match cyanos::read_best_record(&task.best()) {
            Ok(best) => best,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(cyanos::AppError::io(
                    "read PR runtime best evidence",
                    source,
                ));
            }
        };
        if best.task_id() != task.task_id().as_str()
            || best.pr_branch_commit() != head_commit
            || best.eval_path().trim().is_empty()
        {
            return Ok(None);
        }
        let Some(snapshot) = Self::runtime_evaluation_snapshot(&task, best.eval_path())? else {
            return Ok(None);
        };
        let mut evidence = Vec::new();
        evidence.extend(snapshot.structural_evidence().iter().cloned());
        evidence.extend(snapshot.recall_evidence().iter().cloned());
        evidence.extend(snapshot.findings().iter().cloned());
        Ok(Some(RuntimeReviewThreadEvidence {
            eval: best.eval_path().to_owned(),
            fixed_thread_ids: cyanos::fixed_review_thread_ids_from_verifier_evidence(&evidence),
        }))
    }

    fn runtime_evaluation_snapshot(
        task: &cyanos::TaskLayout,
        eval: &str,
    ) -> Result<Option<cyanos::EvaluationSnapshot>, cyanos::AppError> {
        let Some(eval_path) = Self::runtime_evaluation_path(task, eval)? else {
            return Ok(None);
        };
        let content = fs::read_to_string(eval_path)
            .map_err(|source| cyanos::AppError::io("read PR runtime eval evidence", source))?;
        cyanos::EvaluationSnapshot::parse(&content)
            .map(Some)
            .map_err(|source| {
                cyanos::AppError::io(
                    "parse PR runtime eval evidence",
                    io::Error::new(io::ErrorKind::InvalidData, source),
                )
            })
    }

    fn runtime_evaluation_path(
        task: &cyanos::TaskLayout,
        eval: &str,
    ) -> Result<Option<PathBuf>, cyanos::AppError> {
        let eval = eval.trim();
        if eval.is_empty() {
            return Ok(None);
        }
        let eval_path = PathBuf::from(eval);
        let path = if eval_path.is_absolute() {
            eval_path
        } else {
            task.root().join(eval_path)
        };
        if !path.is_file() {
            return Ok(None);
        }
        let task_root = fs::canonicalize(task.root())
            .map_err(|source| cyanos::AppError::io("canonicalize PR runtime task root", source))?;
        let eval_path = fs::canonicalize(path).map_err(|source| {
            cyanos::AppError::io("canonicalize PR runtime eval evidence", source)
        })?;
        if eval_path.starts_with(task_root) {
            Ok(Some(eval_path))
        } else {
            Ok(None)
        }
    }

    fn pr_task_layout(config: &cyanos::ProjectConfig, body: &str) -> Option<cyanos::TaskLayout> {
        let home = cyanos::HomeResolver::resolve().ok()?;
        let project_id = Self::project_id_from_repo(config.repo())?;
        let task_id = cyanos::TaskId::new(Self::pr_task_id_from_body(body)?).ok()?;
        Some(home.project(project_id).task(task_id))
    }

    fn project_id_from_repo(repo: &str) -> Option<cyanos::ProjectId> {
        cyanos::ProjectId::new(repo)
            .ok()
            .or_else(|| cyanos::ProjectId::new(format!("https://{repo}")).ok())
    }

    fn pr_task_id_from_body(body: &str) -> Option<String> {
        let start = body.find(CYANOS_COMMENT_MARKER_TASK_PREFIX)?;
        let value_start = start + CYANOS_COMMENT_MARKER_TASK_PREFIX.len();
        let value = body.get(value_start..)?;
        let end = value.find(CYANOS_COMMENT_MARKER_SUFFIX)?;
        let task_id = value.get(..end)?.trim();
        if task_id.is_empty() {
            None
        } else {
            Some(task_id.to_owned())
        }
    }

    fn review_thread_reply_comment_id<'a>(
        records: &'a [cyanos::ReviewCommentRecord],
        thread_id: &str,
    ) -> Option<&'a str> {
        records
            .iter()
            .find(|record| record.thread_id == thread_id && !record.comment_id.trim().is_empty())
            .map(|record| record.comment_id.as_str())
    }

    fn review_resolution_marker(evidence: &ReviewResolutionEvidence) -> String {
        format!(
            "cyanos:resolution=fixed commit={} eval={}",
            evidence.commit, evidence.eval
        )
    }

    fn reply_to_review_comment_with_program(
        program: &str,
        target: PullRequestReviewTarget<'_>,
        comment_id: &str,
        evidence: &ReviewResolutionEvidence,
    ) -> Result<(), cyanos::AppError> {
        let endpoint = format!(
            "repos/{}/{}/pulls/{}/comments/{comment_id}/replies",
            target.owner, target.repo, target.number
        );
        let body_arg = format!("body={}", Self::review_resolution_marker(evidence));
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let mut args = vec!["api"];
            if let Some(host) = target.host {
                args.extend(["--hostname", host]);
            }
            args.extend(["-X", "POST", endpoint.as_str(), "-f", body_arg.as_str()]);
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &args,
                "reply to fixed pull request review thread",
            )?;
            if output.status.success() {
                return Ok(());
            }
            last_code = output.status.code();
            thread::sleep(RETRY_DELAY);
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh api failed to reply to review comment {comment_id} after {RETRY_ATTEMPTS} attempt(s) with exit code {last_code:?}; next action: check gh review-comment permissions and retry PR readiness"
        ))
        .into())
    }

    fn resolve_review_thread_with_program(
        program: &str,
        host: Option<&str>,
        thread_id: &str,
    ) -> Result<(), cyanos::AppError> {
        let query_arg = format!("query={PR_REVIEW_THREAD_RESOLVE_MUTATION}");
        let thread_arg = format!("thread={thread_id}");
        let mut last_code = None;
        for _attempt in 1..=RETRY_ATTEMPTS {
            let mut args = vec!["api"];
            if let Some(host) = host {
                args.extend(["--hostname", host]);
            }
            args.extend([
                "graphql",
                "-f",
                query_arg.as_str(),
                "-f",
                thread_arg.as_str(),
            ]);
            let output = Self::command_output_retrying_errors(
                program,
                Path::new("."),
                &args,
                "resolve pull request review thread",
            )?;
            if output.status.success() {
                return Ok(());
            }
            last_code = output.status.code();
            thread::sleep(RETRY_DELAY);
        }

        Err(cyanos::AgentRuntimeError::new(format!(
            "gh api graphql failed to resolve review thread {thread_id} after {RETRY_ATTEMPTS} attempt(s) with exit code {last_code:?}; next action: check gh permissions and retry PR readiness"
        ))
        .into())
    }

    fn pr_body(
        task: &cyanos::TaskLayout,
        selected: &SampleRunOutcome,
        task_readme: &str,
    ) -> String {
        format!(
            "{}\n\n## 1. Requirement\n\n{}\n\n## 2. Implementation Summary\n\nPromoted sample {} from Cyanos task {}.\n\n## 3. Architecture and Functional Impact\n\nThe selected sample passed structural verifier gates and was promoted from isolated SSD worktree evidence.\n\n## 4. Verification Method\n\n- Composite score: {}\n- Evaluation: {}\n- Patch: {}\n\n## 5. Test Results\n\nSee CI and Cyanos runtime verifier evidence.",
            cyanos::delivery_pr_marker(task.task_id().as_str()),
            task_readme.trim(),
            selected.sample_id,
            task.task_id().as_str(),
            selected.score.total().as_i64(),
            Self::relative_label(task, &selected.eval_path),
            Self::relative_label(task, &selected.patch_path)
        )
    }

    fn has_git_remote(worktree: &Path) -> bool {
        Self::command_output(
            GIT_PROGRAM,
            worktree,
            &["remote", "get-url", GIT_ORIGIN_REMOTE],
            "check git remote",
        )
        .is_ok_and(|output| output.status.success())
    }

    fn run_git(
        worktree: &Path,
        args: &[&str],
        action: &'static str,
    ) -> Result<(), cyanos::AppError> {
        let output = Self::git_output(worktree, args, action)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(cyanos::AgentRuntimeError::new(format!(
                "git {} failed in {} with exit code {:?}",
                args.join(" "),
                worktree.display(),
                output.status.code()
            ))
            .into())
        }
    }

    fn git_output(
        worktree: &Path,
        args: &[&str],
        action: &'static str,
    ) -> Result<Output, cyanos::AppError> {
        Self::command_output(GIT_PROGRAM, worktree, args, action)
    }

    fn prompt_project_id(&self) -> Result<cyanos::ProjectId, cyanos::AppError> {
        self.write_stderr("Project locator:")
            .map_err(|source| cyanos::AppError::io("write project prompt", source))?;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|source| cyanos::AppError::io("read project locator", source))?;

        cyanos::ProjectId::new(input.trim())
            .map_err(cyanos::CliError::InvalidProjectId)
            .map_err(cyanos::AppError::from)
    }

    fn exit_with_output(&self, response: &str) -> ExitCode {
        match self.write_stdout(response) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                let error = cyanos::AppError::io("write CLI output", error);
                self.exit_with_error(&error)
            }
        }
    }

    fn write_stdout(&self, line: &str) -> io::Result<()> {
        let mut stdout = self.stdout.lock();
        writeln!(stdout, "{line}")
    }

    fn exit_with_error(&self, error: &cyanos::AppError) -> ExitCode {
        let report = self.error_reporter.render(error);

        match self.write_stderr(report.body()) {
            Ok(()) => ExitCode::from(report.exit_status().code()),
            Err(_) => ExitCode::FAILURE,
        }
    }

    fn write_stderr(&self, message: &str) -> io::Result<()> {
        let mut stderr = self.stderr.lock();
        writeln!(stderr, "{message}")
    }
}

impl cyanos::RunTaskBrief for TaskBrief {
    fn body(&self) -> &str {
        self.body()
    }

    fn source_label(&self) -> PathBuf {
        self.source_label()
    }

    fn render_task_source(&self) -> String {
        self.render_task_source()
    }
}

impl cyanos::RunLifecycleHooks for Terminal {
    type TaskBrief = TaskBrief;

    fn load_required_project_config(
        &self,
        project: &cyanos::ProjectLayout,
    ) -> Result<cyanos::ProjectConfig, cyanos::AppError> {
        Self::load_required_project_config(project)
    }

    fn preflight_target_repository(
        &self,
        config: &cyanos::ProjectConfig,
    ) -> Result<(), cyanos::AppError> {
        Self::preflight_target_repository(config)
    }

    fn preflight_runtime_repository(
        &self,
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
    ) -> Result<(), cyanos::AppError> {
        Self::preflight_runtime_repository(project, config)
    }

    fn load_github_task(
        &self,
        config: &cyanos::ProjectConfig,
        task_id: &cyanos::TaskId,
    ) -> Result<Self::TaskBrief, cyanos::AppError> {
        Self::load_github_task(config, task_id)
    }

    fn post_intake_blocker_comment(
        &self,
        command: &cyanos::RunCommand,
        config: &cyanos::ProjectConfig,
        brief: &Self::TaskBrief,
    ) -> Result<Option<String>, cyanos::AppError> {
        let findings = validate_task_brief(brief.body());
        if findings.is_empty() {
            return Ok(None);
        }
        Self::post_blocker_comment(command, config, brief, &findings).map(Some)
    }

    fn publish_runtime_checkpoint(
        &self,
        project: &cyanos::ProjectLayout,
        config: &cyanos::ProjectConfig,
        task_id: &cyanos::TaskId,
        label: &str,
    ) -> Result<String, cyanos::AppError> {
        Self::publish_runtime_checkpoint(project, config, task_id, label)
    }

    fn ensure_runtime_prompt(
        &self,
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
    ) -> Result<(), cyanos::AppError> {
        Self::ensure_runtime_prompt(task, prompts)
    }

    fn runtime_prompt_for_resume(
        &self,
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        next_run_index: usize,
    ) -> Result<String, cyanos::AppError> {
        Self::runtime_prompt_for_resume(task, prompts, next_run_index)
    }

    fn resume_selected_sample_state(
        &self,
        state_lines: &mut Vec<String>,
        context: &cyanos::RunContext<'_>,
        current_prompt: &mut String,
        resume_plan: &cyanos::ResumePlan,
        next_run_index: &mut usize,
    ) -> Result<Option<String>, cyanos::AppError> {
        Self::resume_selected_sample_state(
            state_lines,
            context.invocation(),
            context.project(),
            context.config(),
            context.command(),
            context.task(),
            context.prompts(),
            current_prompt,
            resume_plan,
            context.task_readme(),
            next_run_index,
        )
    }

    fn resume_delivery_pr_state(
        &self,
        state_lines: &mut Vec<String>,
        context: &cyanos::RunContext<'_>,
        current_prompt: &mut String,
        resume_plan: &cyanos::ResumePlan,
    ) -> Result<Option<String>, cyanos::AppError> {
        Self::resume_delivery_pr_state(
            state_lines,
            context.project(),
            context.config(),
            context.command(),
            context.task(),
            context.prompts(),
            current_prompt,
            resume_plan,
        )
    }

    fn task_result_for_resume(
        &self,
        task: &cyanos::TaskLayout,
        resume_plan: &cyanos::ResumePlan,
    ) -> Result<cyanos::TaskResult, cyanos::AppError> {
        Self::task_result_for_resume(task, resume_plan)
    }

    fn run_samples(
        &self,
        batch: &cyanos::RunSampleBatch<'_>,
    ) -> Result<Vec<cyanos::SampleRunOutcome>, cyanos::AppError> {
        Self::run_samples(batch)
    }

    fn record_task_result_run(
        &self,
        task_result: &mut cyanos::TaskResult,
        run_index: usize,
        selected: &cyanos::SampleRunOutcome,
        task: &cyanos::TaskLayout,
    ) {
        Self::record_task_result_run(task_result, run_index, selected, task);
    }

    fn promote_selected_sample(
        &self,
        task: &cyanos::TaskLayout,
        selected: &cyanos::SampleRunOutcome,
    ) -> Result<(), cyanos::AppError> {
        Self::promote_selected_sample(task, selected)
    }

    fn push_and_open_pr(
        &self,
        task: &cyanos::TaskLayout,
        selected: &cyanos::SampleRunOutcome,
        config: &cyanos::ProjectConfig,
        task_readme: &str,
    ) -> Result<String, cyanos::AppError> {
        Self::push_and_open_pr(task, selected, config, task_readme)
    }

    fn continue_after_opened_pr(
        &self,
        state_lines: &mut Vec<String>,
        context: &cyanos::RunContext<'_>,
        current_prompt: &mut String,
        pr: &str,
        run_index: usize,
    ) -> Result<cyanos::PrContinuation, cyanos::AppError> {
        Self::continue_after_opened_pr(
            state_lines,
            context.project(),
            context.config(),
            context.command(),
            context.task(),
            context.prompts(),
            current_prompt,
            pr,
            run_index,
        )
    }

    fn evolve_prompt(
        &self,
        task: &cyanos::TaskLayout,
        prompts: &cyanos::PromptSet,
        current_prompt: &str,
        selected: &cyanos::SampleRunOutcome,
        next_prompt_index: usize,
    ) -> Result<String, cyanos::AppError> {
        Self::evolve_prompt(task, prompts, current_prompt, selected, next_prompt_index)
    }
}

fn current_dir() -> Result<PathBuf, cyanos::AppError> {
    std::env::current_dir().map_err(|source| cyanos::AppError::io("read current directory", source))
}

fn repository_https_remote(repo: &str) -> String {
    let repo = repo.trim_end_matches(GIT_SUFFIX);
    let mut parts = repo.split('/');
    let Some(first) = parts.next() else {
        return format!("https://github.com/{repo}.git");
    };
    let Some(second) = parts.next() else {
        return format!("https://github.com/{repo}.git");
    };

    match (parts.next(), parts.next()) {
        (Some(third), None) => format!("https://{first}/{second}/{third}.git"),
        (None, None) => format!("https://github.com/{first}/{second}.git"),
        _ => format!("https://github.com/{repo}.git"),
    }
}

fn pr_readiness_feedback_from_report(report: &str) -> Option<String> {
    let mut lines = report
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let review_decision = lines.next().unwrap_or_default();
    let merge_state = lines.next().unwrap_or_default();
    let mut details = Vec::new();

    if pr_review_decision_blocks(review_decision) {
        details.push(format!("review_decision={review_decision}"));
    }
    if pr_merge_state_blocks(merge_state) {
        details.push(format!("merge_state={merge_state}"));
    }
    for line in lines {
        if let Some(check) = pr_check_feedback_from_line(line) {
            details.push(check);
        }
    }

    (!details.is_empty()).then(|| {
        format!(
            "PR readiness failed; source=github_pr_readiness {}",
            details.join(" ")
        )
    })
}

fn pr_review_decision_blocks(decision: &str) -> bool {
    let normalized = decision.trim().to_ascii_uppercase();
    !matches!(normalized.as_str(), "" | "APPROVED")
}

fn pr_merge_state_blocks(state: &str) -> bool {
    let normalized = state.trim().to_ascii_uppercase();
    !matches!(normalized.as_str(), "" | "CLEAN" | "HAS_HOOKS")
}

fn pr_check_feedback_from_line(line: &str) -> Option<String> {
    let parts = pr_check_line_parts(line)?;
    let normalized = parts.conclusion.to_ascii_uppercase();
    if matches!(
        normalized.as_str(),
        "" | "SUCCESS" | "PASS" | "PASSED" | "SKIPPED" | "NEUTRAL"
    ) {
        let normalized_status = parts.status.unwrap_or_default().to_ascii_uppercase();
        if matches!(
            normalized_status.as_str(),
            "" | "COMPLETED" | "SUCCESS" | "SKIPPED"
        ) {
            return None;
        }
        let mut feedback = format!(
            "check name={} conclusion=PENDING",
            pr_feedback_token(parts.name)
        );
        let _ = write!(
            feedback,
            " status={}",
            pr_feedback_token(&normalized_status)
        );
        if let Some(url) = parts.url.filter(|value| !value.is_empty()) {
            let _ = write!(feedback, " url={}", pr_feedback_token(url));
        }
        return Some(feedback);
    }

    let mut feedback = format!(
        "check name={} conclusion={normalized}",
        pr_feedback_token(parts.name)
    );
    if let Some(url) = parts.url.filter(|value| !value.is_empty()) {
        let _ = write!(feedback, " url={}", pr_feedback_token(url));
    }
    Some(feedback)
}

struct PrCheckLine<'a> {
    name: &'a str,
    conclusion: &'a str,
    status: Option<&'a str>,
    url: Option<&'a str>,
}

impl<'a> PrCheckLine<'a> {
    const fn new(
        name: &'a str,
        conclusion: &'a str,
        status: Option<&'a str>,
        url: Option<&'a str>,
    ) -> Self {
        Self {
            name,
            conclusion,
            status,
            url,
        }
    }
}

fn pr_check_line_parts(line: &str) -> Option<PrCheckLine<'_>> {
    let tab_parts = line.split('\t').collect::<Vec<_>>();
    match tab_parts.as_slice() {
        [name, conclusion, status, url, ..] => Some(PrCheckLine::new(
            name.trim(),
            conclusion.trim(),
            Some(status.trim()),
            Some(url.trim()),
        )),
        [name, conclusion, third] if third.trim().starts_with("http") => Some(PrCheckLine::new(
            name.trim(),
            conclusion.trim(),
            None,
            Some(third.trim()),
        )),
        [name, conclusion, status] => Some(PrCheckLine::new(
            name.trim(),
            conclusion.trim(),
            Some(status.trim()),
            None,
        )),
        [name, conclusion] => Some(PrCheckLine::new(name.trim(), conclusion.trim(), None, None)),
        _ => line
            .rsplit_once(' ')
            .map(|(name, conclusion)| PrCheckLine::new(name.trim(), conclusion.trim(), None, None)),
    }
}

fn pr_feedback_token(value: &str) -> String {
    cyanos::RuntimeCheckpoint::sanitize_state_value(value)
}

fn repository_permission_allows_delivery(permission: &str) -> bool {
    matches!(permission.trim(), "ADMIN" | "MAINTAIN" | "WRITE")
}

fn repo_graphql_target(repo: &str) -> Option<(Option<String>, String, String)> {
    let normalized = repo
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(GIT_SUFFIX);
    let normalized = normalized
        .split_once("://")
        .map_or(normalized, |(_scheme, rest)| rest);
    let parts = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [owner, name] if !owner.is_empty() && !name.is_empty() => {
            Some((None, (*owner).to_owned(), (*name).to_owned()))
        }
        [host, owner, name] if !host.is_empty() && !owner.is_empty() && !name.is_empty() => Some((
            Some((*host).to_owned()),
            (*owner).to_owned(),
            (*name).to_owned(),
        )),
        _ => None,
    }
}

fn pr_number_from_url(pr: &str) -> Option<String> {
    let trimmed = pr.trim();
    if trimmed.bytes().all(|byte| byte.is_ascii_digit()) && !trimmed.is_empty() {
        return Some(trimmed.to_owned());
    }
    let number = trimmed
        .split_once("/pull/")?
        .1
        .split(['/', '?', '#'])
        .next()?;
    (number.bytes().all(|byte| byte.is_ascii_digit()) && !number.is_empty())
        .then(|| number.to_owned())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TaskBrief {
    repo: String,
    task_id: cyanos::TaskId,
    url: String,
    title: String,
    body: String,
}

impl TaskBrief {
    fn parse(repo: &str, task_id: &cyanos::TaskId, stdout: &[u8]) -> Option<Self> {
        let output = String::from_utf8_lossy(stdout);
        let (url, rest) = output.split_once(TASK_TITLE_MARKER)?;
        let (title, body) = rest.split_once(TASK_BODY_MARKER)?;
        Some(Self {
            repo: repo.to_owned(),
            task_id: task_id.clone(),
            url: url.trim().to_owned(),
            title: title.trim().to_owned(),
            body: body.trim().to_owned(),
        })
    }

    fn body(&self) -> &str {
        &self.body
    }

    fn url(&self) -> &str {
        &self.url
    }

    fn source_label(&self) -> PathBuf {
        PathBuf::from(format!("{}/issues/{}", self.repo, self.task_id.as_str()))
    }

    fn render_task_source(&self) -> String {
        format!(
            "# {}\n\nSource: {}\n\n{}",
            self.title.trim(),
            self.url(),
            self.body.trim()
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TaskBriefFinding {
    message: String,
}

impl TaskBriefFinding {
    fn new(message: String) -> Self {
        Self { message }
    }

    fn message(&self) -> &str {
        &self.message
    }
}

fn validate_task_brief(body: &str) -> Vec<TaskBriefFinding> {
    REQUIRED_TASK_SECTIONS
        .into_iter()
        .filter_map(|section| match markdown_section(body, section) {
            None => Some(TaskBriefFinding::new(format!(
                "Missing required section: {section}"
            ))),
            Some(content) if task_section_is_vague(content) => Some(TaskBriefFinding::new(
                format!("Section is too vague: {section}"),
            )),
            Some(content) if section_has_too_many_ungrouped_bullets(content) => {
                Some(TaskBriefFinding::new(format!(
                    "Section has too many bullets without subheadings: {section}"
                )))
            }
            Some(_) => None,
        })
        .collect()
}

fn markdown_section<'a>(body: &'a str, section: &str) -> Option<&'a str> {
    let mut start = None;
    let mut end = body.len();
    let mut section_level = None;
    let mut offset = 0;

    for line in body.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches(['\r', '\n']);
        let Some(heading) = markdown_heading(line_without_newline) else {
            offset += line.len();
            continue;
        };

        if start.is_some() && section_level.is_some_and(|level| heading.level <= level) {
            end = offset;
            break;
        }

        if heading.title.eq_ignore_ascii_case(section) {
            start = Some(offset + line.len());
            section_level = Some(heading.level);
        }
        offset += line.len();
    }

    start.and_then(|start| body.get(start..end))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MarkdownHeading {
    level: usize,
    title: String,
}

#[cfg(test)]
fn markdown_heading_title(line: &str) -> Option<String> {
    markdown_heading(line).map(|heading| heading.title)
}

fn markdown_heading(line: &str) -> Option<MarkdownHeading> {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    let title = trimmed
        .trim_start_matches('#')
        .trim()
        .trim_end_matches('#')
        .trim();
    if title.is_empty() {
        None
    } else {
        Some(MarkdownHeading {
            level,
            title: title.to_owned(),
        })
    }
}

fn task_section_is_vague(content: &str) -> bool {
    let normalized = content
        .lines()
        .map(|line| line.trim().trim_start_matches(['-', '*']).trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let lowered = normalized.to_ascii_lowercase();

    normalized
        .chars()
        .filter(|character| !character.is_whitespace())
        .count()
        < MIN_TASK_SECTION_CHARS
        || matches!(
            lowered.as_str(),
            "todo" | "tbd" | "n/a" | "na" | "none" | "fix it" | "make it work"
        )
}

fn section_has_too_many_ungrouped_bullets(content: &str) -> bool {
    let mut total_bullets = 0;
    let mut current_group_bullets = 0;
    let mut max_group_bullets = 0;
    let mut has_subheading = false;

    for line in content.lines() {
        if markdown_heading(line).is_some() {
            has_subheading = true;
            max_group_bullets = max_group_bullets.max(current_group_bullets);
            current_group_bullets = 0;
            continue;
        }

        let trimmed = line.trim_start();
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            total_bullets += 1;
            current_group_bullets += 1;
        }
    }
    max_group_bullets = max_group_bullets.max(current_group_bullets);

    total_bullets > TASK_SECTION_MAX_UNGROUPED_BULLETS
        && (!has_subheading || max_group_bullets > TASK_SECTION_MAX_UNGROUPED_BULLETS)
}

fn blocker_comment_body(command: &cyanos::RunCommand, findings: &[TaskBriefFinding]) -> String {
    let mut body = format!(
        "{CYANOS_COMMENT_MARKER_TASK_INTAKE}\nCyanos blocked task intake for `{}` because the issue brief is incomplete or unclear.\n\n",
        command.task_id().as_str()
    );
    body.push_str("Missing or unclear required task brief:\n");
    for finding in findings {
        body.push_str("- ");
        body.push_str(finding.message());
        body.push('\n');
    }
    body.push_str("\nNext action: update the issue body with concrete `Problem`, `Expected Behavior`, `Scope`, `Acceptance Criteria`, and `Verification` sections, then rerun `cyanos run --project ");
    body.push_str(command.project_id().as_str());
    body.push_str(" --task ");
    body.push_str(command.task_id().as_str());
    body.push_str("`.");
    body
}

fn monotonic_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

type SampleRunOutcome = cyanos::SampleRunOutcome;
type PrContinuation = cyanos::PrContinuation;
type SampleBatch<'a> = cyanos::RunSampleBatch<'a>;

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::{
        CYANOS_COMMENT_MARKER_TASK_INTAKE, FixedReviewThreadResolution, OUTER_RUN_LIMIT,
        PullRequestReviewTarget, RUNTIME_PROMPT_FILE, ReviewResolutionEvidence, STATE_BRIEF_FROZEN,
        STATE_EVALUATED_SAMPLES, STATE_GLOBAL_BEST_ACCEPTED, STATE_GLOBAL_BEST_REJECTED,
        STATE_PR_FEEDBACK, STATE_PR_OPENED, STATE_PROMPT_EVOLVED, STATE_RUNNING_SAMPLES,
        STATE_RUNTIME_PUBLISH_FAILED, STATE_TERMINAL_FAILURE, SampleRunOutcome, TASK_BODY_MARKER,
        TASK_TITLE_MARKER, TaskBrief, TaskBriefFinding, Terminal, blocker_comment_body,
        markdown_heading_title, markdown_section, pr_number_from_url,
        pr_readiness_feedback_from_report, repo_graphql_target, repository_https_remote,
        repository_permission_allows_delivery, validate_task_brief,
    };

    use cyanos::{
        ReviewCommentRecord, ReviewThreadEvidence, fixed_review_thread_ids_from_verifier_evidence,
        parse_unresolved_review_thread_count, review_comment_records_from_tsv,
        review_comment_to_finding, review_feedback_from_comment_records,
        review_feedback_from_comment_records_with_evidence, unresolved_fixed_thread_ids,
        unresolved_fixed_thread_ids_with_evidence, unresolved_outdated_thread_ids,
    };

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cyanos-main-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ))
    }

    fn task_layout(root: &Path) -> Result<cyanos::TaskLayout, cyanos::IdentifierError> {
        Ok(cyanos::CyanosHome::new(root.join("home/.cyanos"))
            .project(cyanos::ProjectId::new("demo")?)
            .task(cyanos::TaskId::new("123".to_owned())?))
    }

    fn score(value: i64, tier: cyanos::QualityTier) -> cyanos::ScoreBreakdown {
        cyanos::ScoreBreakdown::new(
            cyanos::SampleScore::new(value),
            tier,
            cyanos::SampleScore::new(value),
            cyanos::SampleScore::new(value),
            cyanos::SampleScore::new(value),
        )
    }

    fn outcome(
        sample_id: usize,
        evaluation: cyanos::Evaluation,
        score: cyanos::ScoreBreakdown,
        task: &cyanos::TaskLayout,
    ) -> SampleRunOutcome {
        SampleRunOutcome {
            run_index: 1,
            sample_id,
            evaluation,
            score,
            eval_path: task.sample_eval(sample_id, 1),
            summary_path: task.sample_summary(sample_id, 1),
            patch_path: task.sample_patch(sample_id, 1),
        }
    }

    fn run_git(cwd: &Path, args: &[&str]) -> std::io::Result<()> {
        let output = Command::new("git").current_dir(cwd).args(args).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "git {} failed with {:?}\nstdout:\n{}\nstderr:\n{}",
                args.join(" "),
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    fn init_git_worktree(worktree: &Path) -> std::io::Result<()> {
        fs::create_dir_all(worktree)?;
        run_git(worktree, &["init"])?;
        run_git(worktree, &["checkout", "-B", "main"])?;
        run_git(worktree, &["config", "user.name", "Cyanos Test"])?;
        run_git(
            worktree,
            &["config", "user.email", "cyanos-test@example.com"],
        )?;
        fs::write(worktree.join("README.md"), "before\n")?;
        run_git(worktree, &["add", "README.md"])?;
        run_git(worktree, &["commit", "-m", "Initial commit"])
    }

    fn write_cargo_project(worktree: &Path, lib_rs: &str) -> std::io::Result<()> {
        fs::write(
            worktree.join("Cargo.toml"),
            "[package]\nname = \"verifier-target\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )?;
        fs::create_dir_all(worktree.join("src"))?;
        fs::write(worktree.join("src/lib.rs"), lib_rs)
    }

    fn write_executable(path: &Path, body: &str) -> std::io::Result<()> {
        let file_name = path
            .file_name()
            .map_or_else(|| "executable".into(), |name| name.to_string_lossy());
        let temp = path.with_file_name(format!(
            ".{file_name}.tmp.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        let mut file = fs::File::create(&temp)?;
        std::io::Write::write_all(&mut file, body.as_bytes())?;
        file.sync_all()?;
        drop(file);
        #[cfg(unix)]
        set_executable(&temp)?;
        fs::rename(temp, path)?;
        Ok(())
    }

    #[cfg(unix)]
    fn set_executable(path: &Path) -> std::io::Result<()> {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
    }

    fn assert_result_error<T, E>(result: Result<T, E>) {
        assert!(result.err().is_some());
    }

    fn json_string_field(line: &str, field: &str) -> Option<String> {
        let needle = format!("\"{field}\":\"");
        let value = line.split_once(&needle)?.1;
        let mut parsed = String::new();
        let mut escaped = false;
        for ch in value.chars() {
            if escaped {
                parsed.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                return Some(parsed);
            } else {
                parsed.push(ch);
            }
        }
        None
    }

    #[test]
    fn validates_required_task_brief_sections() {
        let valid = "## Problem\n\nUsers need a concrete workflow.\n\n## Expected Behavior\n\nThe CLI should run one selected task.\n\n## Scope\n\nOnly task intake changes are in scope.\n\n## Acceptance Criteria\n\nThe agent must not start for incomplete tasks.\n\n## Verification\n\nRun the CLI integration tests.\n";

        assert!(validate_task_brief(valid).is_empty());
        assert_eq!(
            markdown_section(valid, "Scope").map(str::trim),
            Some("Only task intake changes are in scope.")
        );
        let nested = "## Expected Behavior\n\n### CLI\n\n- prints direct output\n\n## Scope\n\nOnly CLI output is in scope.\n";
        assert_eq!(
            markdown_section(nested, "Expected Behavior").map(str::trim),
            Some("### CLI\n\n- prints direct output")
        );

        let findings = validate_task_brief("## Problem\n\nTBD\n");
        let messages = findings
            .iter()
            .map(super::TaskBriefFinding::message)
            .collect::<Vec<_>>();
        assert!(messages.contains(&"Section is too vague: Problem"));
        assert!(messages.contains(&"Missing required section: Expected Behavior"));
        assert!(messages.contains(&"Missing required section: Verification"));
        let dense = "## Problem\n\n- one\n- two\n- three\n- four\n- five\n- six\n\n## Expected Behavior\n\nThe CLI should run one selected task.\n\n## Scope\n\nOnly task intake changes are in scope.\n\n## Acceptance Criteria\n\nThe agent must not start for incomplete tasks.\n\n## Verification\n\nRun the CLI integration tests.\n";
        assert!(
            validate_task_brief(dense)
                .iter()
                .any(|finding| finding.message()
                    == "Section has too many bullets without subheadings: Problem")
        );
        let grouped = "## Problem\n\n### CLI\n\n- one\n- two\n- three\n\n### Runtime\n\n- four\n- five\n- six\n\n## Expected Behavior\n\nThe CLI should run one selected task.\n\n## Scope\n\nOnly task intake changes are in scope.\n\n## Acceptance Criteria\n\nThe agent must not start for incomplete tasks.\n\n## Verification\n\nRun the CLI integration tests.\n";
        assert!(validate_task_brief(grouped).is_empty());

        let overloaded_group = "## Problem\n\n### CLI\n\n- one\n- two\n- three\n- four\n- five\n- six\n\n## Expected Behavior\n\nThe CLI should run one selected task.\n\n## Scope\n\nOnly task intake changes are in scope.\n\n## Acceptance Criteria\n\nThe agent must not start for incomplete tasks.\n\n## Verification\n\nRun the CLI integration tests.\n";
        assert!(
            validate_task_brief(overloaded_group)
                .iter()
                .any(|finding| finding.message()
                    == "Section has too many bullets without subheadings: Problem")
        );
    }

    #[test]
    fn parses_task_brief_from_gh_template_output() -> Result<(), Box<dyn std::error::Error>> {
        let task_id = cyanos::TaskId::new("123".to_owned())?;
        let output = format!(
            "https://github.com/owner/repo/issues/123\n{TASK_TITLE_MARKER}\nImplement task\n{TASK_BODY_MARKER}\n## Problem\n\nThe body comes from GitHub.\n"
        );

        let brief = TaskBrief::parse("owner/repo", &task_id, output.as_bytes())
            .ok_or_else(|| std::io::Error::other("brief should parse"))?;

        assert_eq!(brief.url(), "https://github.com/owner/repo/issues/123");
        assert_eq!(brief.source_label(), PathBuf::from("owner/repo/issues/123"));
        assert!(brief.render_task_source().contains("Implement task"));
        assert!(
            brief
                .render_task_source()
                .contains("The body comes from GitHub.")
        );

        Ok(())
    }

    #[test]
    fn renders_blocker_comment_with_next_action() -> Result<(), Box<dyn std::error::Error>> {
        let command = cyanos::RunCommand::new(
            cyanos::ProjectId::new("demo")?,
            cyanos::TaskId::new("123".to_owned())?,
            cyanos::Agent::default(),
            cyanos::ModelSelection::default(),
            1,
        );
        let findings = validate_task_brief("## Problem\n\nTBD\n");

        let comment = blocker_comment_body(&command, &findings);

        assert!(comment.starts_with(CYANOS_COMMENT_MARKER_TASK_INTAKE));
        assert!(comment.contains("Cyanos blocked task intake"));
        assert!(comment.contains("Section is too vague: Problem"));
        assert!(comment.contains("cyanos run --project demo --task 123"));

        Ok(())
    }

    #[test]
    fn renders_passed_dependency_check_lines() -> Result<(), Box<dyn std::error::Error>> {
        let command = cyanos::RunCommand::new(
            cyanos::ProjectId::new("demo")?,
            cyanos::TaskId::new("123".to_owned())?,
            cyanos::Agent::default(),
            cyanos::ModelSelection::default(),
            1,
        );
        let report = cyanos::DependencyReport::new(vec![
            cyanos::DependencyCheck::new(
                cyanos::DependencyRequirement::GitInstalled,
                vec!["git".to_owned(), "--version".to_owned()],
            ),
            cyanos::DependencyCheck::new_with_attempts(
                cyanos::DependencyRequirement::GitHubCliLoggedIn,
                vec!["gh".to_owned(), "auth".to_owned(), "status".to_owned()],
                2,
            ),
        ]);

        assert_eq!(
            Terminal::dependency_check_lines(&command, &report),
            [
                "cyanos: project=demo task=123 check=✅ name=git_installed",
                "cyanos: project=demo task=123 check=✅ name=gh_logged_in attempts=2"
            ]
        );

        Ok(())
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "github helper fixture exercises multiple readiness paths"
    )]
    fn converts_pr_readiness_failures_into_feedback() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            pr_readiness_feedback_from_report("APPROVED\nCLEAN\nci SUCCESS\n"),
            None
        );
        let feedback =
            pr_readiness_feedback_from_report("CHANGES_REQUESTED\nDIRTY\nci\tFAILURE\tCOMPLETED\thttps://github.com/owner/repo/actions/runs/1/job/2\n")
                .ok_or_else(|| std::io::Error::other("missing readiness feedback"))?;
        assert!(feedback.contains("source=github_pr_readiness"));
        assert!(feedback.contains("review_decision=CHANGES_REQUESTED"));
        assert!(feedback.contains("merge_state=DIRTY"));
        assert!(feedback.contains("check name=ci conclusion=FAILURE"));
        assert!(feedback.contains("url=https://github.com/owner/repo/actions/runs/1/job/2"));
        let distinct_feedback =
            pr_readiness_feedback_from_report("APPROVED\nCLEAN\nRust quality gate\tFAILURE\tCOMPLETED\thttps://github.com/owner/repo/actions/jobs/1\nPull request policy\t\tIN_PROGRESS\thttps://github.com/owner/repo/actions/jobs/2\n")
                .ok_or_else(|| std::io::Error::other("missing distinct check feedback"))?;
        assert!(distinct_feedback.contains("check name=Rust_quality_gate conclusion=FAILURE"));
        assert!(distinct_feedback.contains("url=https://github.com/owner/repo/actions/jobs/1"));
        assert!(distinct_feedback.contains("check name=Pull_request_policy conclusion=PENDING"));
        assert!(distinct_feedback.contains("status=IN_PROGRESS"));
        assert!(distinct_feedback.contains("url=https://github.com/owner/repo/actions/jobs/2"));
        assert_eq!(pr_readiness_feedback_from_report("\n\n"), None);
        assert!(
            pr_readiness_feedback_from_report("APPROVED\nCLEAN\nlegacy check FAILED\n")
                .ok_or_else(|| std::io::Error::other("missing legacy check feedback"))?
                .contains("check name=legacy_check conclusion=FAILED")
        );
        let tabbed_feedback = pr_readiness_feedback_from_report(
            "APPROVED\nCLEAN\nlegacy\tCANCELLED\nqueued\t\tQUEUED\nlinked\tFAILURE\thttps://github.com/owner/repo/actions/jobs/3\n",
        )
        .ok_or_else(|| std::io::Error::other("missing tabbed check feedback"))?;
        assert!(tabbed_feedback.contains("check name=legacy conclusion=CANCELLED"));
        assert!(tabbed_feedback.contains("check name=queued conclusion=PENDING status=QUEUED"));
        assert!(tabbed_feedback.contains(
            "check name=linked conclusion=FAILURE url=https://github.com/owner/repo/actions/jobs/3"
        ));
        assert_eq!(
            cyanos::RuntimeCheckpoint::sanitize_state_value(
                "PR readiness failed; convert CI feedback into another run"
            ),
            "PR_readiness_failed;_convert_CI_feedback_into_another_run"
        );
        assert_eq!(
            repo_graphql_target("owner/repo"),
            Some((None, "owner".to_owned(), "repo".to_owned()))
        );
        assert_eq!(
            repo_graphql_target("github.example.com/owner/repo.git"),
            Some((
                Some("github.example.com".to_owned()),
                "owner".to_owned(),
                "repo".to_owned()
            ))
        );
        assert_eq!(
            pr_number_from_url("https://github.com/owner/repo/pull/456"),
            Some("456".to_owned())
        );
        assert_eq!(pr_number_from_url("456"), Some("456".to_owned()));
        assert_eq!(
            pr_number_from_url("https://github.com/owner/repo/issues/456"),
            None
        );
        assert_eq!(parse_unresolved_review_thread_count(b"2\n")?, 2);
        assert_result_error(parse_unresolved_review_thread_count(b"not-a-number\n"));
        let review_records = review_comment_records_from_tsv(
            b"THREAD1\tfalse\tfalse\tsrc/main.rs\t1001\talice\thttps://github.com/owner/repo/pull/456#discussion_r1\tThis drops the body. Please preserve provenance.\nTHREAD2\tfalse\ttrue\tsrc/lib.rs\t1002\tbob\thttps://github.com/owner/repo/pull/456#discussion_r2\tStale comment\nTHREAD3\tfalse\tfalse\tsrc/lib.rs\t1003\tcyanos-bot\thttps://github.com/owner/repo/pull/456#discussion_r3\tcyanos:resolution=fixed commit=abc123 eval=runs/2/samples/1/eval.json\nTHREAD4\tfalse\tfalse\tsrc/old.rs\t1004\talice\thttps://github.com/owner/repo/pull/456#discussion_r4\tRemove the obsolete old path.\nTHREAD5\tfalse\tfalse\tsrc/main.rs\t1005\talice\thttps://github.com/owner/repo/pull/456#discussion_r5\tThis is not fixed in latest. Please preserve context.\nTHREAD6\tfalse\tfalse\tsrc/main.rs\t1006\talice\thttps://github.com/owner/repo/pull/456#discussion_r6\tOld same-path reviewer concern that the latest judge marked fixed.\n",
        )?;
        assert_eq!(
            unresolved_outdated_thread_ids(&review_records),
            vec!["THREAD2".to_owned()]
        );
        assert_eq!(
            unresolved_fixed_thread_ids(&review_records),
            Vec::<String>::new()
        );
        let current_diff = "diff --git a/src/main.rs b/src/main.rs\n+++ b/src/main.rs\n";
        let managed_marker_evidence =
            ReviewThreadEvidence::default().with_managed_comment_author("cyanos-bot");
        assert_eq!(
            unresolved_fixed_thread_ids_with_evidence(&review_records, managed_marker_evidence),
            vec!["THREAD3".to_owned()]
        );
        let evidence =
            ReviewThreadEvidence::new(current_diff).with_managed_comment_author("cyanos-bot");
        assert_eq!(
            unresolved_fixed_thread_ids_with_evidence(&review_records, evidence),
            vec!["THREAD3".to_owned(), "THREAD4".to_owned()]
        );
        let verifier_fixed_thread_ids = fixed_review_thread_ids_from_verifier_evidence(&[
            "cyanos:review-thread=fixed thread=THREAD6 evidence=judge".to_owned(),
            "cyanos:review-thread=fixed thread=THREAD1 evidence=reviewer".to_owned(),
        ]);
        let verifier_evidence = evidence.with_fixed_thread_ids(&verifier_fixed_thread_ids);
        assert_eq!(
            unresolved_fixed_thread_ids_with_evidence(&review_records, verifier_evidence),
            vec![
                "THREAD3".to_owned(),
                "THREAD4".to_owned(),
                "THREAD6".to_owned()
            ]
        );
        let Some(review_feedback) =
            review_feedback_from_comment_records_with_evidence(&review_records, verifier_evidence)
        else {
            return Err(
                std::io::Error::other("active review comment should produce feedback").into(),
            );
        };
        assert!(review_feedback.contains("code_review_recall=0.00%"));
        assert!(review_feedback.contains("This drops the body. Please preserve provenance."));
        assert!(review_feedback.contains("author=alice marker=external path=src/main.rs"));
        assert!(review_feedback.contains("not fixed in latest"));
        assert!(!review_feedback.contains("Old same-path reviewer concern"));
        assert!(!review_feedback.contains("Stale comment"));
        assert!(!review_feedback.contains("abc123"));
        let unmanaged_feedback = review_feedback_from_comment_records(&review_records)
            .ok_or_else(|| std::io::Error::other("unmanaged marker should remain active"))?;
        assert!(unmanaged_feedback.contains("abc123"));
        let Some(evidence_feedback) = review_feedback_from_comment_records_with_evidence(
            &review_records,
            ReviewThreadEvidence::new(current_diff).with_managed_comment_author("cyanos-bot"),
        ) else {
            return Err(
                std::io::Error::other("path-present comment should produce feedback").into(),
            );
        };
        assert!(!evidence_feedback.contains("src/old.rs"));
        assert!(repository_permission_allows_delivery("WRITE"));
        assert!(repository_permission_allows_delivery("MAINTAIN"));
        assert!(!repository_permission_allows_delivery("READ"));
        assert_eq!(repo_graphql_target("owner"), None);
        let judge_prompt = cyanos::judge_review_prompt(
            "## Problem\n\nImplement it.",
            "diff --git a/file b/file",
            &["cargo_test: passed".to_owned()],
            &["coverage: 100.00%".to_owned()],
        );
        assert!(judge_prompt.contains("Cyanos code review judge"));
        assert!(judge_prompt.contains("CYANOS_JUDGE verdict=<pass|fail>"));
        assert!(judge_prompt.contains("rubric=mvp-2026-05-24"));
        assert!(judge_prompt.contains("risk=<low|medium|high|critical>"));
        assert!(judge_prompt.contains("next=<safe next action>"));
        assert!(judge_prompt.contains("findings=<none|category="));
        let parsed = cyanos::parse_judge_review(
            "noise\nCYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote candidate findings=none summary=meets core requirements\n",
        );
        assert!(parsed.accepted());
        assert_eq!(parsed.score_basis_points(), 10_000);
        assert!(parsed.evidence().contains("recall=100.00%"));
        assert!(parsed.evidence().contains("rubric=mvp-2026-05-24"));
        assert!(parsed.evidence().contains("risk=low"));
        assert!(parsed.evidence().contains("next=promote candidate"));
        assert!(parsed.evidence().contains("findings=[none]"));
        let penalized = cyanos::parse_judge_review(
            "CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=medium next=add tests findings=category=tests,severity=major,concern=coverage gap,fix=add tests,evidence=coverage summary=normalized",
        );
        assert!(penalized.accepted());
        assert_eq!(penalized.score_basis_points(), 5_000);
        let same_category_findings = cyanos::parse_judge_review(
            "CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=medium next=add tests findings=category=tests,severity=major,concern=coverage gap,fix=add tests,evidence=coverage;category=tests,severity=minor,concern=more tests,fix=add edge tests,evidence=coverage summary=normalized",
        );
        assert!(same_category_findings.accepted());
        assert_eq!(same_category_findings.score_basis_points(), 5_000);
        let distinct_category_findings = cyanos::parse_judge_review(
            "CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=medium next=fix both findings=category=tests,severity=major,concern=coverage gap,fix=add tests,evidence=coverage;category=maintainability,severity=major,concern=duplication,fix=dedupe helper,evidence=diff summary=normalized",
        );
        assert!(distinct_category_findings.accepted());
        assert_eq!(distinct_category_findings.score_basis_points(), 2_500);
        let no_summary = cyanos::parse_judge_review(
            "CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote findings=none",
        );
        assert_eq!(no_summary.summary(), "no summary");
        let rejected = cyanos::parse_judge_review(
            "CYANOS_JUDGE verdict=fail rubric=mvp-2026-05-24 risk=high next=implement API findings=category=requirements,severity=blocking,concern=missing API,fix=implement API,evidence=diff summary=missing API",
        );
        assert!(!rejected.accepted());
        assert_eq!(rejected.score_basis_points(), 0);
        assert!(
            rejected
                .findings()
                .first()
                .is_some_and(|finding| finding.to_message().contains("severity=blocking"))
        );
        let invalid_missing_fix = cyanos::parse_judge_review(
            "CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote findings=category=tests,severity=major,concern=coverage gap,evidence=coverage summary=invalid",
        );
        assert!(!invalid_missing_fix.accepted());
        assert_eq!(invalid_missing_fix.score_basis_points(), 0);
        assert!(
            invalid_missing_fix
                .evidence()
                .contains("invalid judge output")
        );
        let invalid_missing_findings = cyanos::parse_judge_review(
            "CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote summary=missing findings",
        );
        assert!(!invalid_missing_findings.accepted());
        assert!(
            invalid_missing_findings
                .evidence()
                .contains("invalid judge output")
        );
        let malformed = cyanos::parse_judge_review("judge skipped");
        assert!(!malformed.accepted());
        assert_eq!(malformed.score_basis_points(), 0);
        Ok(())
    }

    #[test]
    fn normalizes_pr_review_comment_edge_cases() {
        assert_result_error(review_comment_records_from_tsv(b"not\tenough\n"));
        assert_result_error(review_comment_records_from_tsv(
            b"THREAD\tmaybe\tfalse\tsrc/main.rs\t1001\talice\thttps://example.test\tbody\n",
        ));

        let base_record = ReviewCommentRecord {
            thread_id: "THREAD".to_owned(),
            comment_id: "1001".to_owned(),
            is_resolved: false,
            is_outdated: false,
            path: "src/main.rs".to_owned(),
            author: "alice".to_owned(),
            url: "https://github.com/owner/repo/pull/1#discussion_r1".to_owned(),
            body: String::new(),
        };
        let empty = review_comment_to_finding(&base_record);
        assert_eq!(empty.severity(), "blocking");
        assert!(empty.concern().contains("invalid empty PR review comment"));

        let mut structured = base_record.clone();
        structured.body = "cyanos: judge finding category=security,severity=minor,concern=secret leak,fix=remove secret,evidence=diff".to_owned();
        let structured_finding = review_comment_to_finding(&structured);
        assert_eq!(structured_finding.category(), "security");
        assert_eq!(structured_finding.severity(), "minor");
        assert!(structured_finding.evidence_ref().contains("marker=cyanos"));
        assert!(structured_finding.evidence_ref().contains("diff"));

        let mut invalid_structured = base_record.clone();
        invalid_structured.body = "severity=major concern=gap fix=fix evidence=diff".to_owned();
        let invalid_structured_finding = review_comment_to_finding(&invalid_structured);
        assert!(
            invalid_structured_finding
                .concern()
                .contains("invalid structured PR review comment")
        );
        invalid_structured.body =
            "category=requirements severity=major concern=gap fix=fix evidence=diff".to_owned();
        assert!(
            review_comment_to_finding(&invalid_structured)
                .fix()
                .contains("comma-separated")
        );

        let mut plain = base_record.clone();
        plain.body = "Reviewer found a regression".to_owned();
        let plain_finding = review_comment_to_finding(&plain);
        assert_eq!(
            plain_finding.fix(),
            "address reviewer feedback in code and tests"
        );

        let mut long_fix = base_record;
        long_fix.body = format!("Please {}", "x".repeat(200));
        let long_fix_finding = review_comment_to_finding(&long_fix);
        assert!(long_fix_finding.fix().ends_with("..."));

        let mut resolved = structured;
        resolved.is_resolved = true;
        assert_eq!(review_feedback_from_comment_records(&[resolved]), None);

        let mut fixed = plain;
        fixed.body = "Addressed in deadbeef with current verifier evidence".to_owned();
        assert_eq!(
            unresolved_fixed_thread_ids(&[fixed.clone()]),
            Vec::<String>::new()
        );
        assert!(review_feedback_from_comment_records(&[fixed.clone()]).is_some());
        fixed.body =
            "cyanos:resolution=fixed commit=deadbeef eval=runs/1/samples/1/eval.json".to_owned();
        assert_eq!(
            unresolved_fixed_thread_ids(&[fixed.clone()]),
            Vec::<String>::new()
        );
        assert!(review_feedback_from_comment_records(&[fixed.clone()]).is_some());
        fixed.author = "cyanos".to_owned();
        assert_eq!(
            unresolved_fixed_thread_ids(&[fixed.clone()]),
            Vec::<String>::new()
        );
        assert!(review_feedback_from_comment_records(&[fixed.clone()]).is_some());
        fixed.author = "cyanos-bot".to_owned();
        let managed_evidence =
            ReviewThreadEvidence::default().with_managed_comment_author("cyanos-bot");
        assert_eq!(
            unresolved_fixed_thread_ids_with_evidence(&[fixed.clone()], managed_evidence),
            vec!["THREAD"]
        );
        assert_eq!(
            review_feedback_from_comment_records_with_evidence(&[fixed], managed_evidence),
            None
        );
        assert_eq!(
            fixed_review_thread_ids_from_verifier_evidence(&[
                "cyanos:review-thread=fixed thread=THREAD evidence=verifier".to_owned(),
                "cyanos:review-thread=fixed thread=SPOOF evidence=reviewer".to_owned(),
                "thread=LOOSE evidence=judge".to_owned(),
                "cyanos:review-thread=fixed evidence=judge".to_owned(),
            ]),
            vec!["THREAD"]
        );
    }

    #[test]
    fn retries_command_spawn_errors_before_reporting() -> Result<(), Box<dyn std::error::Error>> {
        let Err(error) = Terminal::command_output_retrying_errors(
            "definitely-not-a-cyanos-command",
            Path::new("."),
            &[],
            "run missing helper",
        ) else {
            return Err(std::io::Error::other("missing command should fail").into());
        };

        assert!(error.to_string().contains("run missing helper"));
        Ok(())
    }

    #[test]
    fn covers_error_helper_edges() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("helper-edges");
        fs::create_dir_all(&root)?;

        assert_result_error(Terminal::select_sample(&[]));
        assert_result_error(Terminal::write_sample_patch(
            &root,
            &root.join("patch.diff"),
        ));
        assert_eq!(cyanos::VerifierCommand::all().len(), 3);
        let review = cyanos::JudgeReview::passed_with_metadata("", "", "", "ok", Vec::new());
        assert!(review.evidence().contains("rubric=mvp-2026-05-24"));
        assert!(run_git(&root, &["definitely-not-a-git-command"]).is_err());

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "github helper fixture exercises task, comment, permission, and readiness paths"
    )]
    fn github_helpers_use_injected_cli_program() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("github-helper");
        fs::create_dir_all(&root)?;
        let gh = root.join("gh-ok");
        write_executable(
            &gh,
            "#!/bin/sh\nif [ \"$1\" = \"repo\" ] && [ \"$2\" = \"view\" ]; then printf '%s\\n' 'WRITE'; exit 0; fi\nif [ \"$1\" = \"repo\" ] && [ \"$2\" = \"create\" ]; then exit 0; fi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"graphql\" ]; then printf '%s\\n' 'Issue'; exit 0; fi\nif [ \"$1\" = \"issue\" ] && [ \"$2\" = \"view\" ]; then\n  printf 'https://github.com/owner/repo/issues/%s\\n' \"$3\"\n  printf '%s\\n' '---CYANOS-TITLE---'\n  printf '%s\\n' 'Task title'\n  printf '%s\\n' '---CYANOS-BODY---'\n  printf '%s\\n' '## Problem'\n  printf '%s\\n' 'Users need the helper tested.'\n  exit 0\nfi\nif [ \"$1\" = \"issue\" ] && [ \"$2\" = \"comment\" ]; then printf 'https://github.com/owner/repo/issues/%s#issuecomment-1\\n' \"$3\"; exit 0; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then printf '%s\\n' 'CHANGES_REQUESTED' 'DIRTY' 'ci FAILURE'; exit 0; fi\nexit 1\n",
        )?;
        let config = cyanos::ProjectConfig::new(
            "owner/repo".to_owned(),
            "owner/cyanos-repo".to_owned(),
            "main".to_owned(),
            root.join("source"),
        );
        let task_id = cyanos::TaskId::new("123".to_owned())?;
        let command = cyanos::RunCommand::new(
            cyanos::ProjectId::new("demo")?,
            task_id.clone(),
            cyanos::Agent::default(),
            cyanos::ModelSelection::default(),
            1,
        );

        assert!(Terminal::github_repo_exists_with_program(
            gh.to_string_lossy().as_ref(),
            "owner/cyanos-repo"
        )?);
        Terminal::create_runtime_github_repo_with_program(
            gh.to_string_lossy().as_ref(),
            "owner/cyanos-repo",
        )?;
        Terminal::require_repository_write_permission(
            gh.to_string_lossy().as_ref(),
            "owner/repo",
            "target repository",
        )?;
        let gh_read_only = root.join("gh-read-only");
        write_executable(
            &gh_read_only,
            "#!/bin/sh\nif [ \"$1\" = \"repo\" ] && [ \"$2\" = \"view\" ]; then printf '%s\\n' 'READ'; exit 0; fi\nexit 1\n",
        )?;
        let permission_result = Terminal::require_repository_write_permission(
            gh_read_only.to_string_lossy().as_ref(),
            "owner/repo",
            "target repository",
        );
        assert_result_error(permission_result);
        let brief = Terminal::load_github_task_with_program(
            gh.to_string_lossy().as_ref(),
            &config,
            &task_id,
        )?;
        assert_eq!(brief.url(), "https://github.com/owner/repo/issues/123");
        let gh_pull_request_task = root.join("gh-pr-task");
        write_executable(
            &gh_pull_request_task,
            "#!/bin/sh\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"graphql\" ]; then printf '%s\\n' 'PullRequest'; exit 0; fi\nexit 1\n",
        )?;
        let pr_task_result = Terminal::load_github_task_with_program(
            gh_pull_request_task.to_string_lossy().as_ref(),
            &config,
            &task_id,
        );
        assert_result_error(pr_task_result);
        let comment = Terminal::post_blocker_comment_with_program(
            gh.to_string_lossy().as_ref(),
            &command,
            &config,
            &brief,
            &[TaskBriefFinding::new(
                "Missing required section: Scope".to_owned(),
            )],
        )?;
        assert!(comment.ends_with("#issuecomment-1"));
        assert!(
            Terminal::pr_readiness_feedback_with_program(
                gh.to_string_lossy().as_ref(),
                "https://github.com/owner/repo/pull/456",
                &config,
            )?
            .is_some()
        );
        let gh_threads = root.join("gh-review-threads");
        write_executable(
            &gh_threads,
            "#!/bin/sh\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then printf '%s\\n' 'APPROVED' 'CLEAN' 'ci SUCCESS'; exit 0; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"diff\" ]; then printf '%s\\n' 'diff --git a/src/main.rs b/src/main.rs' '+++ b/src/main.rs'; exit 0; fi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"graphql\" ]; then\n  case \"$*\" in *viewer*) printf '%s\\n' 'cyanos-bot'; exit 0;; *resolveReviewThread*) exit 0;; esac\n  printf '%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' 'THREAD1' 'false' 'false' 'src/main.rs' '1001' 'reviewer' 'https://github.com/owner/repo/pull/456#discussion_r1' 'This drops review context. Please preserve the actual body.'\n  printf '%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' 'THREAD2' 'false' 'true' 'src/lib.rs' '1002' 'reviewer' 'https://github.com/owner/repo/pull/456#discussion_r2' 'Stale comment'\n  exit 0\nfi\nexit 1\n",
        )?;
        let thread_feedback = Terminal::pr_readiness_feedback_with_program(
            gh_threads.to_string_lossy().as_ref(),
            "https://github.com/owner/repo/pull/456",
            &config,
        )?;
        assert_eq!(
            thread_feedback.as_deref(),
            Some(
                "PR readiness failed; source=github_review code_review_recall=0.00% rule=severity_penalty_v1 blocking=10000 major=5000 minor=2500 note=1000 category=worst_finding findings=[category=requirements severity=blocking concern=This drops review context. Please preserve the actual body. fix=Please preserve the actual body. evidence=github_review url=https://github.com/owner/repo/pull/456#discussion_r1 author=reviewer marker=external path=src/main.rs]\njudge finding category=requirements severity=blocking concern=This drops review context. Please preserve the actual body. fix=Please preserve the actual body. evidence=github_review url=https://github.com/owner/repo/pull/456#discussion_r1 author=reviewer marker=external path=src/main.rs"
            )
        );
        let gh_threads_clean = root.join("gh-review-threads-clean");
        write_executable(
            &gh_threads_clean,
            "#!/bin/sh\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then printf '%s\\n' 'APPROVED' 'CLEAN' 'ci SUCCESS'; exit 0; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"diff\" ]; then exit 0; fi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"graphql\" ]; then case \"$*\" in *viewer*) printf '%s\\n' 'cyanos-bot';; esac; exit 0; fi\nif [ \"$1\" = \"api\" ]; then exit 0; fi\nexit 1\n",
        )?;
        assert_eq!(
            Terminal::pr_readiness_feedback_with_program(
                gh_threads_clean.to_string_lossy().as_ref(),
                "https://github.com/owner/repo/pull/456",
                &config,
            )?,
            None
        );
        let review_log = root.join("review-log");
        let gh_threads_fixed = root.join("gh-review-threads-fixed");
        write_executable(
            &gh_threads_fixed,
            &format!(
                "#!/bin/sh\nLOG='{}'\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n  case \"$*\" in *headRefOid,body*) printf '%s\\n' 'dc310ea' '## 4. Verification Method' '- Evaluation: runs/2/samples/1/eval.json'; exit 0;; esac\n  printf '%s\\n' 'APPROVED' 'CLEAN' 'ci SUCCESS'; exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"diff\" ]; then printf '%s\\n' 'diff --git a/src/main.rs b/src/main.rs' '+++ b/src/main.rs'; exit 0; fi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"graphql\" ]; then\n  case \"$*\" in *viewer*) printf '%s\\n' 'cyanos-bot'; exit 0;; *resolveReviewThread*) exit 0;; esac\n  printf '%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' 'THREAD4' 'false' 'false' 'src/old.rs' '1004' 'reviewer' 'https://github.com/owner/repo/pull/456#discussion_r4' 'Remove obsolete old path.'\n  exit 0\nfi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"-X\" ] && [ \"$3\" = \"POST\" ]; then exit 0; fi\nexit 1\n",
                review_log.display()
            ),
        )?;
        assert_eq!(
            Terminal::pr_readiness_feedback_with_program(
                gh_threads_fixed.to_string_lossy().as_ref(),
                "https://github.com/owner/repo/pull/456",
                &config,
            )?,
            None
        );
        let review_commands = fs::read_to_string(&review_log)?;
        let reply_index = review_commands
            .find("POST repos/owner/repo/pulls/456/comments/1004/replies")
            .ok_or_else(|| std::io::Error::other("missing fixed-thread reply"))?;
        let resolve_index = review_commands
            .find("resolveReviewThread")
            .ok_or_else(|| std::io::Error::other("missing fixed-thread resolve"))?;
        assert!(reply_index < resolve_index);
        assert!(review_commands.contains(
            "body=cyanos:resolution=fixed commit=dc310ea eval=runs/2/samples/1/eval.json"
        ));
        let same_path_task_id = format!("same_path_{}", super::monotonic_nanos());
        let same_path_task = cyanos::HomeResolver::resolve()?
            .project(cyanos::ProjectId::new("owner/repo")?)
            .task(cyanos::TaskId::new(same_path_task_id.clone())?);
        let same_path_eval = same_path_task.sample_eval(1, 1);
        fs::create_dir_all(
            same_path_eval
                .parent()
                .ok_or_else(|| std::io::Error::other("missing eval parent"))?,
        )?;
        fs::write(
            &same_path_eval,
            cyanos::Evaluation::accepted_with_findings(vec![cyanos::EvaluationFinding::new(
                "cyanos:review-thread=fixed thread=THREAD1 evidence=judge".to_owned(),
            )])
            .to_json(),
        )?;
        cyanos::write_best_record(
            &same_path_task.best(),
            &cyanos::BestRecord::new(
                &same_path_task_id,
                1,
                1,
                score(10_000, cyanos::QualityTier::Passed),
                "runs/1/samples/1/eval.json",
                "runs/1/samples/1/patch.diff",
                "target-commit",
                "cyanos-commit",
                "dc310ea",
            ),
        )?;
        let same_path_log = root.join("same-path-review-log");
        let gh_threads_same_path_fixed = root.join("gh-review-threads-same-path-fixed");
        write_executable(
            &gh_threads_same_path_fixed,
            &format!(
                "#!/bin/sh\nLOG='{}'\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n  case \"$*\" in *headRefOid,body*) printf '%s\\n' 'dc310ea' '<!--cyanos:task={}-->' '- Evaluation: runs/1/samples/1/eval.json'; exit 0;; esac\n  printf '%s\\n' 'APPROVED' 'CLEAN' 'ci SUCCESS'; exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"diff\" ]; then printf '%s\\n' 'diff --git a/src/main.rs b/src/main.rs' '+++ b/src/main.rs'; exit 0; fi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"graphql\" ]; then\n  case \"$*\" in *viewer*) printf '%s\\n' 'cyanos-bot'; exit 0;; *resolveReviewThread*) exit 0;; esac\n  printf '%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' 'THREAD1' 'false' 'false' 'src/main.rs' '1001' 'reviewer' 'https://github.com/owner/repo/pull/456#discussion_r1' 'Same-path concern fixed by selected judge evidence.'\n  exit 0\nfi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"-X\" ] && [ \"$3\" = \"POST\" ]; then exit 0; fi\nexit 1\n",
                same_path_log.display(),
                same_path_task_id
            ),
        )?;
        assert_eq!(
            Terminal::pr_readiness_feedback_with_program(
                gh_threads_same_path_fixed.to_string_lossy().as_ref(),
                "https://github.com/owner/repo/pull/456",
                &config,
            )?,
            None
        );
        let same_path_commands = fs::read_to_string(&same_path_log)?;
        let same_path_reply_index = same_path_commands
            .find("POST repos/owner/repo/pulls/456/comments/1001/replies")
            .ok_or_else(|| std::io::Error::other("missing same-path fixed-thread reply"))?;
        let same_path_resolve_index = same_path_commands
            .find("resolveReviewThread")
            .ok_or_else(|| std::io::Error::other("missing same-path fixed-thread resolve"))?;
        assert!(same_path_reply_index < same_path_resolve_index);
        assert!(same_path_commands.contains("body=cyanos:resolution=fixed commit=dc310ea eval="));
        let spoof_task_id = format!("spoof_{}", super::monotonic_nanos());
        let source_eval = config.source_path().join("runs/1/samples/1/eval.json");
        fs::create_dir_all(
            source_eval
                .parent()
                .ok_or_else(|| std::io::Error::other("missing source eval parent"))?,
        )?;
        fs::write(
            &source_eval,
            cyanos::Evaluation::accepted_with_findings(vec![cyanos::EvaluationFinding::new(
                "cyanos:review-thread=fixed thread=THREAD1 evidence=judge".to_owned(),
            )])
            .to_json(),
        )?;
        let spoof_log = root.join("spoof-review-log");
        let gh_threads_spoof = root.join("gh-review-threads-spoof");
        write_executable(
            &gh_threads_spoof,
            &format!(
                "#!/bin/sh\nLOG='{}'\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n  case \"$*\" in *headRefOid,body*) printf '%s\\n' 'dc310ea' '<!--cyanos:task={}-->' 'cyanos:review-thread=fixed thread=THREAD1 evidence=judge' '- Evaluation: runs/1/samples/1/eval.json'; exit 0;; esac\n  printf '%s\\n' 'APPROVED' 'CLEAN' 'ci SUCCESS'; exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"diff\" ]; then printf '%s\\n' 'diff --git a/src/main.rs b/src/main.rs' '+++ b/src/main.rs'; exit 0; fi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"graphql\" ]; then\n  case \"$*\" in *viewer*) printf '%s\\n' 'cyanos-bot'; exit 0;; *resolveReviewThread*) exit 0;; esac\n  printf '%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' 'THREAD1' 'false' 'false' 'src/main.rs' '1001' 'reviewer' 'https://github.com/owner/repo/pull/456#discussion_r1' 'Source-tree eval spoof should remain active.'\n  exit 0\nfi\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"-X\" ] && [ \"$3\" = \"POST\" ]; then exit 0; fi\nexit 1\n",
                spoof_log.display(),
                spoof_task_id
            ),
        )?;
        let spoof_feedback = Terminal::pr_readiness_feedback_with_program(
            gh_threads_spoof.to_string_lossy().as_ref(),
            "https://github.com/owner/repo/pull/456",
            &config,
        )?
        .ok_or_else(|| std::io::Error::other("spoofed body evidence should stay active"))?;
        assert!(spoof_feedback.contains("Source-tree eval spoof should remain active."));
        let spoof_commands = fs::read_to_string(&spoof_log)?;
        assert!(!spoof_commands.contains("comments/1001/replies"));
        assert!(!spoof_commands.contains("resolveReviewThread"));

        fs::remove_dir_all(same_path_task.root())?;
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn github_helpers_report_retry_exhaustion() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("github-helper-fail");
        fs::create_dir_all(&root)?;
        let gh = root.join("gh-fail");
        write_executable(&gh, "#!/bin/sh\nexit 7\n")?;
        let config = cyanos::ProjectConfig::new(
            "owner/repo".to_owned(),
            "owner/cyanos-repo".to_owned(),
            "main".to_owned(),
            root.join("source"),
        );
        let task_id = cyanos::TaskId::new("123".to_owned())?;
        let brief = TaskBrief {
            repo: "owner/repo".to_owned(),
            task_id: task_id.clone(),
            url: "https://github.com/owner/repo/issues/123".to_owned(),
            title: "Task".to_owned(),
            body: "Body".to_owned(),
        };
        let command = cyanos::RunCommand::new(
            cyanos::ProjectId::new("demo")?,
            task_id.clone(),
            cyanos::Agent::default(),
            cyanos::ModelSelection::default(),
            1,
        );

        assert!(!Terminal::github_repo_exists_with_program(
            gh.to_string_lossy().as_ref(),
            "owner/cyanos-repo"
        )?);
        let create_result = Terminal::create_runtime_github_repo_with_program(
            gh.to_string_lossy().as_ref(),
            "owner/cyanos-repo",
        );
        assert_result_error(create_result);
        let load_result = Terminal::load_github_task_with_program(
            gh.to_string_lossy().as_ref(),
            &config,
            &task_id,
        );
        assert_result_error(load_result);
        let comment_result = Terminal::post_blocker_comment_with_program(
            gh.to_string_lossy().as_ref(),
            &command,
            &config,
            &brief,
            &[],
        );
        assert_result_error(comment_result);
        let permission_result = Terminal::require_repository_write_permission(
            gh.to_string_lossy().as_ref(),
            "owner/repo",
            "target repository",
        );
        assert_result_error(permission_result);
        let readiness_result = Terminal::pr_readiness_feedback_with_program(
            gh.to_string_lossy().as_ref(),
            "https://github.com/owner/repo/pull/456",
            &config,
        );
        assert_result_error(readiness_result);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn review_resolution_helpers_cover_edges() -> Result<(), Box<dyn std::error::Error>> {
        let parsed =
            Terminal::parse_pr_resolution_evidence(b"abc123\n## Body\nNo evaluation line\n")?;
        assert_eq!(
            parsed,
            ReviewResolutionEvidence {
                commit: "abc123".to_owned(),
                eval: "pr-readiness-review-evidence".to_owned(),
                fixed_thread_ids: Vec::new()
            }
        );
        assert_eq!(
            Terminal::pr_evaluation_path_from_body("- Evaluation: `runs/1/eval.json`").as_deref(),
            Some("runs/1/eval.json")
        );
        assert_eq!(Terminal::pr_evaluation_path_from_body("No eval"), None);
        assert_result_error(Terminal::parse_pr_resolution_evidence(b"\n"));
        assert_eq!(
            Terminal::review_thread_reply_comment_id(&[], "THREAD"),
            None
        );

        let root = temp_root("review-resolution-helper");
        fs::create_dir_all(&root)?;
        let gh_actor = root.join("gh-actor");
        write_executable(
            &gh_actor,
            "#!/bin/sh\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"--hostname\" ] && [ \"$4\" = \"graphql\" ]; then printf '%s\\n' 'cyanos-bot'; exit 0; fi\nexit 1\n",
        )?;
        assert_eq!(
            Terminal::github_actor_with_program(
                gh_actor.to_string_lossy().as_ref(),
                Some("github.example.com")
            )?,
            "cyanos-bot"
        );

        let gh_empty_actor = root.join("gh-empty-actor");
        write_executable(&gh_empty_actor, "#!/bin/sh\nexit 0\n")?;
        assert_result_error(Terminal::github_actor_with_program(
            gh_empty_actor.to_string_lossy().as_ref(),
            None,
        ));

        let gh_reply_fail = root.join("gh-reply-fail");
        write_executable(&gh_reply_fail, "#!/bin/sh\nexit 7\n")?;
        let target = PullRequestReviewTarget {
            host: Some("github.example.com"),
            owner: "owner",
            repo: "repo",
            number: "456",
            pr: "https://github.com/owner/repo/pull/456",
        };
        assert_result_error(Terminal::reply_to_review_comment_with_program(
            gh_reply_fail.to_string_lossy().as_ref(),
            target,
            "1004",
            &ReviewResolutionEvidence {
                commit: "abc123".to_owned(),
                eval: "runs/1/eval.json".to_owned(),
                fixed_thread_ids: Vec::new(),
            },
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "review resolution edge coverage needs several fake gh fixtures"
    )]
    fn review_resolution_controller_edges() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("review-resolution-controller");
        fs::create_dir_all(&root)?;
        let config = cyanos::ProjectConfig::new(
            "owner/repo".to_owned(),
            "owner/cyanos-repo".to_owned(),
            "main".to_owned(),
            root.join("source"),
        );
        let invalid_repo_config = cyanos::ProjectConfig::new(
            "owner".to_owned(),
            "owner/cyanos-repo".to_owned(),
            "main".to_owned(),
            root.join("source"),
        );
        assert_result_error(Terminal::pr_unresolved_review_feedback_with_program(
            "unused",
            "https://github.com/owner/repo/pull/456",
            &invalid_repo_config,
        ));
        assert_result_error(Terminal::pr_unresolved_review_feedback_with_program(
            "unused",
            "https://github.com/owner/repo/issues/456",
            &config,
        ));

        let gh_review_fail = root.join("gh-review-fail");
        write_executable(&gh_review_fail, "#!/bin/sh\nexit 7\n")?;
        let host_config = cyanos::ProjectConfig::new(
            "github.example.com/owner/repo".to_owned(),
            "owner/cyanos-repo".to_owned(),
            "main".to_owned(),
            root.join("source"),
        );
        assert_result_error(Terminal::pr_unresolved_review_feedback_with_program(
            gh_review_fail.to_string_lossy().as_ref(),
            "https://github.example.com/owner/repo/pull/456",
            &host_config,
        ));
        assert_result_error(Terminal::pr_resolution_evidence_with_program(
            gh_review_fail.to_string_lossy().as_ref(),
            "https://github.com/owner/repo/pull/456",
            &config,
        ));
        assert_eq!(
            Terminal::pr_evaluation_path_from_body("- Evaluation: "),
            None
        );

        let target = PullRequestReviewTarget {
            host: None,
            owner: "owner",
            repo: "repo",
            number: "456",
            pr: "https://github.com/owner/repo/pull/456",
        };
        let resolution_evidence = ReviewResolutionEvidence {
            commit: "abc123".to_owned(),
            eval: "runs/1/eval.json".to_owned(),
            fixed_thread_ids: Vec::new(),
        };
        let missing_comment = vec![ReviewCommentRecord {
            thread_id: "THREAD".to_owned(),
            comment_id: String::new(),
            is_resolved: false,
            is_outdated: false,
            path: "src/main.rs".to_owned(),
            author: "reviewer".to_owned(),
            url: "https://github.com/owner/repo/pull/456#discussion_r1".to_owned(),
            body: "fixed by diff".to_owned(),
        }];
        let thread_ids = vec!["THREAD".to_owned()];
        assert_result_error(Terminal::reply_and_resolve_fixed_review_threads(
            "unused",
            target,
            FixedReviewThreadResolution {
                records: &missing_comment,
                managed_actor: "cyanos-bot",
                thread_ids: &thread_ids,
                evidence: &resolution_evidence,
            },
        ));

        let gh_resolve = root.join("gh-resolve");
        write_executable(
            &gh_resolve,
            "#!/bin/sh\nif [ \"$1\" = \"api\" ] && [ \"$2\" = \"graphql\" ]; then exit 0; fi\nexit 1\n",
        )?;
        let mut managed_record = missing_comment
            .first()
            .cloned()
            .ok_or_else(|| std::io::Error::other("missing fixture record"))?;
        managed_record.author = "cyanos-bot".to_owned();
        managed_record.body =
            "cyanos:resolution=fixed commit=abc123 eval=runs/1/eval.json".to_owned();
        let managed_marker = vec![managed_record];
        Terminal::reply_and_resolve_fixed_review_threads(
            gh_resolve.to_string_lossy().as_ref(),
            target,
            FixedReviewThreadResolution {
                records: &managed_marker,
                managed_actor: "cyanos-bot",
                thread_ids: &thread_ids,
                evidence: &resolution_evidence,
            },
        )?;

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn parses_coverage_total_as_basis_points() {
        let output = "Filename Regions Missed Cover Functions Missed Cover Lines Missed Cover\nTOTAL 10 0 100.00% 4 0 100.00% 20 1 95.50%\n";

        assert_eq!(cyanos::parse_coverage_basis_points(output), Some(9_550));
        assert_eq!(cyanos::parse_coverage_basis_points("no total"), None);
        assert_eq!(cyanos::format_coverage_evidence(9_550), "coverage: 95.50%");
        assert_eq!(
            cyanos::format_coverage_evidence(12_000),
            "coverage: 100.00%"
        );
        assert_eq!(
            cyanos::parse_coverage_basis_points("TOTAL 1 0 100.999%"),
            Some(10_000)
        );
        assert_eq!(
            cyanos::parse_coverage_basis_points("TOTAL 1 0 1.2%"),
            Some(120)
        );
        assert_eq!(
            cyanos::parse_coverage_basis_points("TOTAL 1 0 not-a-percent"),
            None
        );
        assert_eq!(
            cyanos::parse_coverage_basis_points("TOTAL 1 0 1.2.3%"),
            None
        );
    }

    #[test]
    fn measures_coverage_with_profile_environment_short_circuit()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("coverage-short-circuit");
        fs::create_dir_all(&root)?;

        let coverage = Terminal::measure_coverage(&root)?;

        if std::env::var_os(super::LLVM_PROFILE_FILE_ENV).is_some()
            && std::env::var_os(super::FORCE_COVERAGE_VERIFIER_ENV).is_none()
        {
            assert_eq!(coverage, super::PERFECT_SAMPLE_SCORE);
        } else {
            assert_eq!(coverage, 0);
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn parses_markdown_heading_edge_cases() {
        assert_eq!(
            markdown_heading_title("### Scope ###").as_deref(),
            Some("Scope")
        );
        assert_eq!(markdown_heading_title("plain text"), None);
        assert_eq!(markdown_heading_title("###"), None);
        assert!(
            validate_task_brief("## Problem\n\n- make it work\n")
                .iter()
                .any(|finding| finding.message() == "Section is too vague: Problem")
        );
    }

    #[test]
    fn selects_accepted_sample_before_higher_rejected_score()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("select-sample");
        let task = task_layout(&root)?;
        let rejected = outcome(
            1,
            cyanos::Evaluation::rejected(vec![cyanos::EvaluationFinding::new(
                "tests failed".to_owned(),
            )]),
            score(9_999, cyanos::QualityTier::TestFailed),
            &task,
        );
        let accepted = outcome(
            2,
            cyanos::Evaluation::accepted(),
            score(100, cyanos::QualityTier::Passed),
            &task,
        );

        let outcomes = [rejected, accepted];
        let selected = Terminal::select_sample(&outcomes)?;

        assert_eq!(selected.sample_id, 2);
        if Terminal::select_sample(&[]).is_ok() {
            return Err(std::io::Error::other("expected empty outcomes to fail").into());
        }
        Ok(())
    }

    #[test]
    fn records_task_result_feedback_for_selected_sample() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = temp_root("record-result");
        let task = task_layout(&root)?;
        let accepted = outcome(
            1,
            cyanos::Evaluation::accepted(),
            score(10_000, cyanos::QualityTier::Passed),
            &task,
        );
        let rejected = outcome(
            2,
            cyanos::Evaluation::rejected(vec![cyanos::EvaluationFinding::new(
                "lint failed".to_owned(),
            )]),
            score(3_000, cyanos::QualityTier::TestFailed),
            &task,
        );
        let mut result = Terminal::new_task_result();

        Terminal::record_task_result_run(&mut result, 1, &accepted, &task);
        Terminal::record_task_result_run(&mut result, 2, &rejected, &task);
        let json = result.to_json();

        assert!(json.contains("\"failure_class\": \"none\""));
        assert!(json.contains("promote verified candidate"));
        assert!(json.contains("\"failure_class\": \"verification\""));
        assert!(json.contains("revise prompt and rerun samples"));
        Ok(())
    }

    #[test]
    #[expect(
        clippy::expect_used,
        reason = "test fixture asserts appended run existence"
    )]
    fn hydrates_task_result_before_resume_append() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("resume-result");
        let task = task_layout(&root)?;
        fs::create_dir_all(task.root())?;
        let first = outcome(
            1,
            cyanos::Evaluation::accepted(),
            score(8_000, cyanos::QualityTier::Passed),
            &task,
        );
        let second = outcome(
            2,
            cyanos::Evaluation::accepted(),
            score(7_500, cyanos::QualityTier::PartialSuccess),
            &task,
        );
        let mut result = Terminal::new_task_result();
        Terminal::record_task_result_run(&mut result, 1, &first, &task);
        fs::write(task.result(), result.to_json())?;
        let resume_plan = cyanos::ResumePlan {
            state: cyanos::ResumeState::PrFeedback,
            last_run_index: 1,
            next_run_index: 2,
            delivery_pr: None,
        };

        let mut hydrated = Terminal::task_result_for_resume(&task, &resume_plan)?;
        Terminal::record_task_result_run(&mut hydrated, 2, &second, &task);

        assert_eq!(hydrated.runs().len(), 2);
        let appended = hydrated.runs().get(1).expect("missing appended run");
        assert_eq!(appended.previous_score().total().as_i64(), 8_000);
        assert_eq!(hydrated.best_score().total().as_i64(), 8_000);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn hydrates_selected_sample_for_mid_run_resume() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("resume-selected");
        let task = task_layout(&root)?;
        fs::create_dir_all(task.root())?;
        let run_index = 2;
        let selected = SampleRunOutcome {
            run_index,
            sample_id: 2,
            evaluation: cyanos::Evaluation::accepted_with_findings(vec![
                cyanos::EvaluationFinding::new("review passed".to_owned()),
            ])
            .with_evidence(
                vec!["cargo_test: passed".to_owned()],
                vec!["judge: 90%".to_owned()],
            ),
            score: score(9_000, cyanos::QualityTier::Passed),
            eval_path: task.sample_eval(2, run_index),
            summary_path: task.sample_summary(2, run_index),
            patch_path: task.sample_patch(2, run_index),
        };
        fs::create_dir_all(
            selected
                .eval_path
                .parent()
                .ok_or_else(|| std::io::Error::other("missing eval parent"))?,
        )?;
        fs::write(&selected.eval_path, selected.evaluation.to_json())?;
        fs::write(&selected.summary_path, "summary")?;
        fs::write(&selected.patch_path, "")?;
        let mut result = Terminal::new_task_result();
        Terminal::record_task_result_run(&mut result, run_index, &selected, &task);
        fs::write(task.result(), result.to_json())?;

        let resumed = Terminal::selected_outcome_for_resume(&task, run_index)?;

        assert_eq!(resumed.sample_id, 2);
        assert!(resumed.evaluation.is_accepted());
        assert_eq!(resumed.score.total().as_i64(), 9_000);
        assert_eq!(resumed.eval_path, selected.eval_path);
        assert_eq!(resumed.summary_path, selected.summary_path);
        assert_eq!(resumed.patch_path, selected.patch_path);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn resumes_rejected_selected_sample_from_persisted_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("resume-rejected-selected");
        let home = cyanos::CyanosHome::new(root.join("home/.cyanos"));
        let project_id = cyanos::ProjectId::new("demo")?;
        let task_id = cyanos::TaskId::new("123".to_owned())?;
        let project = home.project(project_id.clone());
        let task = project.task(task_id.clone());
        fs::create_dir_all(task.root())?;
        let selected = SampleRunOutcome {
            run_index: 1,
            sample_id: 1,
            evaluation: cyanos::Evaluation::rejected(vec![cyanos::EvaluationFinding::new(
                "coverage missing".to_owned(),
            )])
            .with_evidence(
                vec!["cargo_test: passed".to_owned()],
                vec!["coverage: 0%".to_owned()],
            ),
            score: score(2_000, cyanos::QualityTier::PartialSuccess),
            eval_path: task.sample_eval(1, 1),
            summary_path: task.sample_summary(1, 1),
            patch_path: task.sample_patch(1, 1),
        };
        fs::create_dir_all(
            selected
                .eval_path
                .parent()
                .ok_or_else(|| std::io::Error::other("missing eval parent"))?,
        )?;
        fs::write(&selected.eval_path, selected.evaluation.to_json())?;
        fs::write(&selected.summary_path, "rejected sample")?;
        fs::write(&selected.patch_path, "")?;
        let mut result = Terminal::new_task_result();
        Terminal::record_task_result_run(&mut result, 1, &selected, &task);
        fs::write(task.result(), result.to_json())?;
        let command = cyanos::RunCommand::new(
            project_id,
            task_id,
            cyanos::Agent::default(),
            cyanos::ModelSelection::default(),
            1,
        );
        let invocation = cyanos::Invocation::new(cyanos::CliCommand::Run(command.clone()));
        let config = cyanos::ProjectConfig::new(
            "owner/repo".to_owned(),
            "owner/runtime".to_owned(),
            "main".to_owned(),
            root.join("source"),
        );
        let prompts = cyanos::PromptSet::new(
            cyanos::OuterPrompt::new("outer".to_owned()),
            cyanos::InnerPrompt::new("inner".to_owned()),
        );
        let resume_plan = cyanos::ResumePlan {
            state: cyanos::ResumeState::SampleEvaluation,
            last_run_index: 1,
            next_run_index: 1,
            delivery_pr: None,
        };
        let mut state_lines = Vec::new();
        let mut current_prompt = "inner".to_owned();
        let mut next_run_index = resume_plan.next_run_index;

        let response = Terminal::resume_selected_sample_state(
            &mut state_lines,
            &invocation,
            &project,
            &config,
            &command,
            &task,
            &prompts,
            &mut current_prompt,
            &resume_plan,
            "## Task",
            &mut next_run_index,
        )?;

        assert!(response.is_none());
        assert_eq!(next_run_index, 2);
        assert!(current_prompt.contains("coverage missing"));
        assert!(task.prompt_snapshot(1).is_file());
        let ledger = fs::read_to_string(task.ledger())?;
        assert!(ledger.contains(STATE_GLOBAL_BEST_REJECTED));
        assert!(ledger.contains(STATE_PROMPT_EVOLVED));
        assert!(
            state_lines
                .join("\n")
                .contains("state=resumed_selected_sample run=1 selected_sample=1")
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn selected_resume_reports_invalid_artifacts() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("resume-selected-errors");
        let task = task_layout(&root)?;
        fs::create_dir_all(task.root())?;

        assert_result_error(Terminal::selected_outcome_for_resume(&task, 1));
        fs::write(task.result(), "{not json")?;
        assert_result_error(Terminal::selected_outcome_for_resume(&task, 1));

        fs::write(task.result(), Terminal::new_task_result().to_json())?;
        assert_result_error(Terminal::selected_outcome_for_resume(&task, 1));

        let selected = outcome(
            1,
            cyanos::Evaluation::accepted(),
            score(8_000, cyanos::QualityTier::Passed),
            &task,
        );
        let mut result = Terminal::new_task_result();
        Terminal::record_task_result_run(&mut result, 1, &selected, &task);
        fs::write(task.result(), result.to_json())?;
        assert_result_error(Terminal::selected_outcome_for_resume(&task, 1));

        fs::create_dir_all(
            selected
                .eval_path
                .parent()
                .ok_or_else(|| std::io::Error::other("missing eval parent"))?,
        )?;
        fs::write(&selected.eval_path, "{bad eval")?;
        fs::write(&selected.summary_path, "summary")?;
        fs::write(&selected.patch_path, "")?;
        assert_result_error(Terminal::selected_outcome_for_resume(&task, 1));
        assert_eq!(
            Terminal::resume_artifact_path(&task, "/tmp/cyanos-eval.json"),
            PathBuf::from("/tmp/cyanos-eval.json")
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn plans_resume_from_durable_runtime_evidence() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("resume-plan");
        let task = task_layout(&root)?;
        fs::create_dir_all(task.root())?;
        cyanos::RuntimeCheckpoint::append_ledger(&task, STATE_BRIEF_FROZEN, "frozen")?;
        cyanos::RuntimeCheckpoint::append_ledger(&task, STATE_PR_FEEDBACK, "needs changes")?;
        fs::write(
            task.result(),
            "{\n  \"status\": \"improved\",\n  \"runs\": [\n    {\n      \"run_index\": 1,\n      \"selected_sample\": 1,\n      \"status\": \"improved\",\n      \"feedback\": {\n        \"eval_path\": \"runs/1/samples/1/eval.json\",\n        \"next_action\": \"continue\"\n      }\n    },\n    {\n      \"run_index\": 2,\n      \"selected_sample\": 1,\n      \"status\": \"improved\",\n      \"feedback\": {\n        \"eval_path\": \"runs/2/samples/1/eval.json\",\n        \"next_action\": \"continue\"\n      }\n    }\n  ]\n}\n",
        )?;
        cyanos::write_delivery_pr_identity(
            &task,
            &cyanos::DeliveryPrIdentity::new(
                "https://github.com/owner/repo/pull/456",
                "feature/installable-alpha",
            ),
        )?;

        let plan = Terminal::plan_resume(&task)?;

        assert_eq!(
            cyanos::RuntimeCheckpoint::last_ledger_state(&task)?.as_deref(),
            Some(STATE_PR_FEEDBACK)
        );
        assert_eq!(
            cyanos::TaskResultSnapshot::parse(&fs::read_to_string(task.result())?)?.max_run_index(),
            Some(2)
        );
        assert_eq!(
            cyanos::resume_state_from_ledger_state(STATE_PR_FEEDBACK),
            cyanos::ResumeState::PrFeedback
        );
        assert_eq!(plan.state, cyanos::ResumeState::PrFeedback);
        assert_eq!(plan.last_run_index, 2);
        assert_eq!(plan.next_run_index, 3);
        let delivery_pr = plan
            .delivery_pr
            .ok_or_else(|| std::io::Error::other("missing delivery PR"))?;
        assert_eq!(delivery_pr.url, "https://github.com/owner/repo/pull/456");
        assert_eq!(delivery_pr.head, "feature/installable-alpha");
        assert_eq!(
            cyanos::delivery_pr_identity_from_text(
                "https://github.com/owner/repo/pull/1\n",
                "feature/123"
            )
            .ok_or_else(|| std::io::Error::other("missing legacy delivery PR"))?
            .head,
            "feature/123"
        );
        let candidates = cyanos::delivery_pr_candidates_from_tsv(
            b"https://github.com/owner/repo/pull/1\tfeature/a\tCyanos task 123\t<!--cyanos:task=123-->\nhttps://github.com/owner/repo/pull/1\tfeature/a\tCyanos task 123\t<!--cyanos:task=123-->\n",
            "feature/123",
            "123",
        );
        assert_eq!(candidates.len(), 1);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "resume helper coverage exercises several state mappings and parser edges together"
    )]
    fn resume_helpers_cover_prompt_and_parser_edges() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("resume-helpers");
        let task = task_layout(&root)?;
        fs::create_dir_all(task.root())?;
        let prompts = cyanos::PromptSet::new(
            cyanos::OuterPrompt::new("outer".to_owned()),
            cyanos::InnerPrompt::new("inner".to_owned()),
        );

        Terminal::ensure_runtime_prompt(&task, &prompts)?;
        Terminal::ensure_runtime_prompt(&task, &prompts)?;
        assert_eq!(
            Terminal::runtime_prompt_for_resume(&task, &prompts, 1)?,
            "inner"
        );
        fs::remove_file(task.root().join(RUNTIME_PROMPT_FILE))?;
        fs::write(task.prompt_snapshot(2), "snapshot prompt")?;
        assert_eq!(
            Terminal::runtime_prompt_for_resume(&task, &prompts, 3)?,
            "snapshot prompt"
        );
        fs::remove_file(task.prompt_snapshot(2))?;
        assert_eq!(
            Terminal::runtime_prompt_for_resume(&task, &prompts, 3)?,
            "inner"
        );

        assert_eq!(cyanos::ResumeState::New.as_str(), "new");
        assert_eq!(
            cyanos::ResumeState::SampleExecution.as_str(),
            "sample_execution"
        );
        assert_eq!(
            cyanos::ResumeState::SampleEvaluation.as_str(),
            "evaluated_samples"
        );
        assert_eq!(
            cyanos::ResumeState::PromptEvolution.as_str(),
            "prompt_evolution"
        );
        assert_eq!(cyanos::ResumeState::Promotion.as_str(), "promotion");
        assert_eq!(cyanos::ResumeState::PrOpened.as_str(), "pr_opened");
        assert_eq!(
            cyanos::ResumeState::TerminalFailure.as_str(),
            "terminal_failure"
        );
        assert_eq!(
            cyanos::next_run_index_for_resume(cyanos::ResumeState::New, 2),
            1
        );
        assert_eq!(
            cyanos::next_run_index_for_resume(cyanos::ResumeState::SampleExecution, 0),
            1
        );
        assert_eq!(
            cyanos::next_run_index_for_resume(cyanos::ResumeState::PromptEvolution, 2),
            3
        );
        assert_eq!(
            cyanos::next_run_index_for_resume(cyanos::ResumeState::SampleEvaluation, 2),
            2
        );
        assert_eq!(
            cyanos::next_run_index_for_resume(cyanos::ResumeState::Promotion, 2),
            2
        );
        assert_eq!(
            cyanos::next_run_index_for_resume(cyanos::ResumeState::PrReady, 0),
            1
        );
        assert_eq!(
            cyanos::resume_state_from_ledger_state(STATE_BRIEF_FROZEN),
            cyanos::ResumeState::Intake
        );
        assert_eq!(
            cyanos::resume_state_from_ledger_state(STATE_RUNNING_SAMPLES),
            cyanos::ResumeState::SampleExecution
        );
        assert_eq!(
            cyanos::resume_state_from_ledger_state(STATE_EVALUATED_SAMPLES),
            cyanos::ResumeState::SampleEvaluation
        );
        assert_eq!(
            cyanos::resume_state_from_ledger_state(STATE_PROMPT_EVOLVED),
            cyanos::ResumeState::PromptEvolution
        );
        assert_eq!(
            cyanos::resume_state_from_ledger_state(STATE_GLOBAL_BEST_ACCEPTED),
            cyanos::ResumeState::Promotion
        );
        assert_eq!(
            cyanos::resume_state_from_ledger_state(STATE_GLOBAL_BEST_REJECTED),
            cyanos::ResumeState::SampleEvaluation
        );
        assert_eq!(
            cyanos::resume_state_from_ledger_state(STATE_PR_OPENED),
            cyanos::ResumeState::PrOpened
        );
        assert_eq!(
            cyanos::resume_state_from_ledger_state(STATE_TERMINAL_FAILURE),
            cyanos::ResumeState::TerminalFailure
        );
        assert_eq!(
            json_string_field("{\"state\":\"pr_\\\"ready\"}", "state").as_deref(),
            Some("pr_\"ready")
        );
        assert_eq!(
            json_string_field("{\"state\":\"unterminated", "state"),
            None
        );
        assert_eq!(
            cyanos::delivery_pr_candidates_from_tsv(
                b"https://github.com/owner/repo/pull/2\t\tFix #123\t\nhttps://github.com/owner/repo/pull/3\tfeature/wrong\tFix #1234\t\n",
                "feature/123",
                "123"
            )
            .first()
            .map(|candidate| candidate.head.as_str()),
            Some("feature/123")
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn evolves_prompt_and_appends_escaped_ledger() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("evolve-prompt");
        let task = task_layout(&root)?;
        fs::create_dir_all(task.root())?;
        let selected = outcome(
            1,
            cyanos::Evaluation::rejected(vec![cyanos::EvaluationFinding::new(
                "coverage missing".to_owned(),
            )]),
            score(0, cyanos::QualityTier::PartialSuccess),
            &task,
        );
        let prompts = cyanos::PromptSet::new(
            cyanos::OuterPrompt::new("outer".to_owned()),
            cyanos::InnerPrompt::new("inner".to_owned()),
        );

        let revised = Terminal::evolve_prompt(&task, &prompts, "inner", &selected, 1)?;
        let revised_from_feedback =
            Terminal::evolve_prompt_from_feedback(&task, &prompts, &revised, "CI failed", 2)?;
        cyanos::RuntimeCheckpoint::append_ledger(
            &task,
            "prompt_evolved",
            "quoted \"summary\"\nnext",
        )?;

        assert!(revised.contains("coverage missing"));
        assert!(revised_from_feedback.contains("CI failed"));
        assert_eq!(
            fs::read_to_string(task.root().join("prompt.md"))?,
            revised_from_feedback
        );
        assert_eq!(fs::read_to_string(task.prompt_snapshot(1))?, revised);
        assert_eq!(
            fs::read_to_string(task.prompt_snapshot(2))?,
            revised_from_feedback
        );
        let ledger = fs::read_to_string(task.ledger())?;
        assert!(ledger.contains("quoted \\\"summary\\\"\\nnext"));
        assert_eq!(
            cyanos::RuntimeCheckpoint::escape_json_fragment("\\\n\r\t\""),
            "\\\\\\n\\r\\t\\\""
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn validates_project_config_before_run() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("config-validation");
        let project = cyanos::CyanosHome::new(root.join("home/.cyanos"))
            .project(cyanos::ProjectId::new("demo")?);
        fs::create_dir_all(project.root())?;

        let missing = Terminal::load_required_project_config(&project);
        if missing.is_ok() {
            return Err(std::io::Error::other("expected missing config to fail").into());
        }
        fs::write(
            project.config(),
            "repo=unknown/unknown\nruntime_repo=unknown/cyanos-unknown\nbase_branch=main\nsource_path=/tmp/source\n",
        )?;
        let unknown = Terminal::load_required_project_config(&project);
        if unknown.is_ok() {
            return Err(std::io::Error::other("expected unknown repo to fail").into());
        }
        fs::write(
            project.config(),
            "repo=owner/repo\nruntime_repo=owner/cyanos-repo\nbase_branch=main\nsource_path=/tmp/source\n",
        )?;
        assert_eq!(
            Terminal::load_required_project_config(&project)?.repo(),
            "owner/repo"
        );
        assert_eq!(
            repository_https_remote("owner/cyanos-repo"),
            "https://github.com/owner/cyanos-repo.git"
        );
        assert_eq!(
            repository_https_remote("github.example.com/owner/cyanos-repo"),
            "https://github.example.com/owner/cyanos-repo.git"
        );
        assert_eq!(
            repository_https_remote("owner"),
            "https://github.com/owner.git"
        );
        assert_eq!(
            repository_https_remote("host/owner/repo/extra"),
            "https://github.com/host/owner/repo/extra.git"
        );
        let invalid_runtime = cyanos::ProjectConfig::new(
            "owner/repo".to_owned(),
            "owner/repo".to_owned(),
            "main".to_owned(),
            PathBuf::from("/tmp/source"),
        );
        let invalid_runtime_result =
            Terminal::preflight_runtime_repository(&project, &invalid_runtime);
        assert_result_error(invalid_runtime_result);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "runtime checkpoint fixture covers success, terminal, and push-failure paths"
    )]
    fn publishes_runtime_checkpoint_to_runtime_repository() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = temp_root("runtime-checkpoint");
        fs::create_dir_all(&root)?;
        let project = cyanos::CyanosHome::new(root.join("home/.cyanos"))
            .project(cyanos::ProjectId::new("demo")?);
        fs::create_dir_all(project.root())?;
        let remote = root.join("runtime.git");
        run_git(
            &root,
            &["init", "--bare", remote.to_string_lossy().as_ref()],
        )?;
        let config = cyanos::ProjectConfig::new(
            "owner/repo".to_owned(),
            "owner/cyanos-repo".to_owned(),
            "main".to_owned(),
            root.join("source"),
        );
        let task_id = cyanos::TaskId::new("123".to_owned())?;

        Terminal::ensure_runtime_git_repository(&project, &config)?;
        Terminal::ensure_runtime_git_repository(&project, &config)?;
        run_git(
            &project.root(),
            &[
                "remote",
                "set-url",
                "origin",
                remote.to_string_lossy().as_ref(),
            ],
        )?;
        fs::write(project.root().join("evidence.txt"), "ok\n")?;
        let tag = Terminal::publish_runtime_checkpoint(&project, &config, &task_id, "unit-test")?;

        assert_eq!(tag, "task-123-unit-test");
        run_git(
            &remote,
            &["show-ref", "--verify", "refs/tags/task-123-unit-test"],
        )?;
        let task = project.task(task_id.clone());
        fs::create_dir_all(task.root())?;
        let command = cyanos::RunCommand::new(
            cyanos::ProjectId::new("demo")?,
            task_id.clone(),
            cyanos::Agent::default(),
            cyanos::ModelSelection::default(),
            1,
        );
        let selected = outcome(
            1,
            cyanos::Evaluation::rejected(vec![cyanos::EvaluationFinding::new(
                "still rejected".to_owned(),
            )]),
            score(1_000, cyanos::QualityTier::PartialSuccess),
            &task,
        );
        let prompts = cyanos::PromptSet::new(
            cyanos::OuterPrompt::new("outer".to_owned()),
            cyanos::InnerPrompt::new("inner".to_owned()),
        );
        let mut state_lines = Vec::new();
        let mut current_prompt = "inner".to_owned();
        let mut next_run_index = OUTER_RUN_LIMIT;
        let terminal = Terminal::resume_rejected_selected_sample(
            &mut state_lines,
            &project,
            &config,
            &command,
            &task,
            &prompts,
            &mut current_prompt,
            &mut next_run_index,
            &selected,
            OUTER_RUN_LIMIT,
        );
        assert_result_error(terminal);
        assert!(fs::read_to_string(task.ledger())?.contains(STATE_TERMINAL_FAILURE));
        assert!(state_lines.join("\n").contains("state=runtime_pushed"));
        run_git(
            &remote,
            &[
                "show-ref",
                "--verify",
                "refs/tags/task-123-terminal-failure",
            ],
        )?;
        run_git(
            &project.root(),
            &["remote", "set-url", "origin", "/missing.git"],
        )?;
        let state_line_count = state_lines.len();
        let checkpoint_result =
            Terminal::publish_runtime_checkpoint(&project, &config, &task_id, "push-fails");
        assert_result_error(checkpoint_result);
        assert_eq!(state_lines.len(), state_line_count);
        let ledger = fs::read_to_string(task.ledger())?;
        assert!(ledger.contains(STATE_RUNTIME_PUBLISH_FAILED));
        assert!(ledger.contains(STATE_TERMINAL_FAILURE));
        assert!(ledger.contains("checkpoint=push-fails"));
        assert!(ledger.contains("remote=owner/cyanos-repo"));
        assert!(ledger.contains("next_action=check runtime repository permissions and rerun"));
        let runtime_git_result = Terminal::run_runtime_git(
            &project,
            config.runtime_repo(),
            &["definitely-not-a-git-command"],
            "bad runtime git",
        );
        assert_result_error(runtime_git_result);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn writes_patch_and_promotes_without_remote() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("promote-no-remote");
        let task = task_layout(&root)?;
        init_git_worktree(&task.worktree())?;
        let patch_path = task.sample_patch(1, 1);
        fs::create_dir_all(
            patch_path
                .parent()
                .ok_or_else(|| std::io::Error::other("missing patch parent"))?,
        )?;
        fs::write(task.worktree().join("README.md"), "after\n")?;
        fs::create_dir_all(task.worktree().join("target"))?;
        fs::write(task.worktree().join("target/ignored.txt"), "ignored\n")?;
        fs::write(task.worktree().join("sample.profraw"), "ignored\n")?;

        Terminal::write_sample_patch(&task.worktree(), &patch_path)?;
        let patch = fs::read_to_string(&patch_path)?;
        assert!(patch.contains("-before"));
        assert!(!patch.contains("target/ignored.txt"));
        assert!(!patch.contains("sample.profraw"));
        run_git(&task.worktree(), &["restore", "README.md"])?;
        fs::remove_dir_all(task.worktree().join("target"))?;
        fs::remove_file(task.worktree().join("sample.profraw"))?;

        let selected = outcome(
            1,
            cyanos::Evaluation::accepted(),
            score(10_000, cyanos::QualityTier::Passed),
            &task,
        );
        let config = cyanos::ProjectConfig::new(
            "owner/repo".to_owned(),
            "owner/cyanos-repo".to_owned(),
            "main".to_owned(),
            task.worktree(),
        );
        Terminal::promote_selected_sample(&task, &selected)?;
        let pr = Terminal::push_and_open_pr(&task, &selected, &config, "## Task");

        assert_result_error(pr);
        assert_eq!(
            fs::read_to_string(task.worktree().join("README.md"))?,
            "after\n"
        );
        let best = fs::read_to_string(task.best())?;
        assert!(best.contains("\"selected_run\": 1"));
        assert!(best.contains("\"selected_sample\": 1"));
        assert!(best.contains("\"eval_path\": \"runs/1/samples/1/eval.json\""));
        assert!(best.contains("\"patch_path\": \"runs/1/samples/1/patch.diff\""));
        run_git(&task.worktree(), &["rev-parse", "--verify", "feature/123"])?;
        Terminal::commit_if_changed(&task.worktree(), "No changes")?;
        let bad_patch = task.sample_patch(2, 1);
        fs::create_dir_all(
            bad_patch
                .parent()
                .ok_or_else(|| std::io::Error::other("missing patch parent"))?,
        )?;
        fs::write(&bad_patch, "not a patch\n")?;
        let apply_result = Terminal::apply_patch_to_worktree(&task, &bad_patch);
        assert_result_error(apply_result);
        let patch_result = Terminal::write_sample_patch(root.join("non-git").as_path(), &bad_patch);
        assert_result_error(patch_result);
        let run_git_result = Terminal::run_git(
            &task.worktree(),
            &["definitely-not-a-git-command"],
            "bad git",
        );
        assert_result_error(run_git_result);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn sample_retry_reset_restores_base_and_removes_partial_artifacts()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("sample-retry-reset");
        let worktree = root.join("sample");
        init_git_worktree(&worktree)?;
        let base = Terminal::current_head(&worktree)?;
        fs::write(worktree.join("README.md"), "failed attempt\n")?;
        fs::write(worktree.join("partial.txt"), "failed artifact\n")?;
        run_git(&worktree, &["add", "README.md"])?;
        run_git(&worktree, &["commit", "-m", "Failed attempt"])?;

        Terminal::reset_sample_worktree(&worktree, &base)?;

        assert_eq!(fs::read_to_string(worktree.join("README.md"))?, "before\n");
        assert!(!worktree.join("partial.txt").exists());
        assert_eq!(Terminal::current_head(&worktree)?, base);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "PR helper fixture covers persisted, branch, adopted, ambiguous, and create paths"
    )]
    fn opens_or_updates_pull_requests_with_injected_gh() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("open-pr");
        fs::create_dir_all(&root)?;
        let task = task_layout(&root)?;
        let selected = outcome(
            1,
            cyanos::Evaluation::accepted(),
            score(10_000, cyanos::QualityTier::Passed),
            &task,
        );
        let config = cyanos::ProjectConfig::new(
            "owner/repo".to_owned(),
            "owner/cyanos-repo".to_owned(),
            "main".to_owned(),
            root.join("source"),
        );
        let existing = root.join("gh-existing");
        let edit_log = root.join("pr-edit.log");
        write_executable(
            &existing,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then printf '%s\\n' 'https://github.com/owner/repo/pull/7'; exit 0; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"edit\" ]; then printf '%s\\n' \"$*\" > '{}'; exit 0; fi\nexit 1\n",
                edit_log.display()
            ),
        )?;
        let create = root.join("gh-create");
        write_executable(
            &create,
            "#!/bin/sh\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then exit 1; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then exit 0; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"create\" ]; then printf '%s\\n' 'https://github.com/owner/repo/pull/8'; exit 0; fi\nexit 1\n",
        )?;
        let fail = root.join("gh-fail-pr");
        write_executable(
            &fail,
            "#!/bin/sh\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then exit 1; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then exit 0; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"create\" ]; then exit 9; fi\nexit 1\n",
        )?;
        let adopt = root.join("gh-adopt-pr");
        let adopt_log = root.join("pr-adopt.log");
        write_executable(
            &adopt,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then exit 1; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then printf '%s\\t%s\\t%s\\t%s\\n' 'https://github.com/owner/repo/pull/9' 'feature/installable-alpha' 'Cyanos task 123' '<!--cyanos:task=123-->'; exit 0; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"edit\" ]; then printf '%s\\n' \"$*\" > '{}'; exit 0; fi\nexit 1\n",
                adopt_log.display()
            ),
        )?;
        let ambiguous = root.join("gh-ambiguous-pr");
        write_executable(
            &ambiguous,
            "#!/bin/sh\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then exit 1; fi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then printf '%s\\t%s\\t%s\\t%s\\n' 'https://github.com/owner/repo/pull/9' 'feature/a' 'Fix #123' ''; printf '%s\\t%s\\t%s\\t%s\\n' 'https://github.com/owner/repo/pull/10' 'feature/b' 'Cyanos task 123' '<!--cyanos:task=123-->'; exit 0; fi\nexit 1\n",
        )?;

        assert_eq!(
            Terminal::open_or_update_pr_with_program(
                existing.to_string_lossy().as_ref(),
                &task,
                &selected,
                &config,
                "## Task"
            )?,
            "https://github.com/owner/repo/pull/7"
        );
        let edited = fs::read_to_string(&edit_log)?;
        assert!(edited.contains("pr edit https://github.com/owner/repo/pull/7"));
        assert!(edited.contains("--repo owner/repo"));
        assert!(edited.contains("--title Cyanos task 123"));
        fs::remove_file(cyanos::delivery_pr_identity_path(&task))?;
        assert_eq!(
            Terminal::open_or_update_pr_with_program(
                create.to_string_lossy().as_ref(),
                &task,
                &selected,
                &config,
                "## Task"
            )?,
            "https://github.com/owner/repo/pull/8"
        );
        assert!(
            fs::read_to_string(cyanos::delivery_pr_identity_path(&task))?
                .contains("https://github.com/owner/repo/pull/8")
        );
        fs::remove_file(cyanos::delivery_pr_identity_path(&task))?;
        assert_eq!(
            Terminal::open_or_update_pr_with_program(
                adopt.to_string_lossy().as_ref(),
                &task,
                &selected,
                &config,
                "## Task"
            )?,
            "https://github.com/owner/repo/pull/9"
        );
        let adopted_identity = fs::read_to_string(cyanos::delivery_pr_identity_path(&task))?;
        assert!(adopted_identity.contains("url=https://github.com/owner/repo/pull/9"));
        assert!(adopted_identity.contains("head=feature/installable-alpha"));
        assert!(
            fs::read_to_string(&adopt_log)?
                .contains("pr edit https://github.com/owner/repo/pull/9")
        );
        fs::remove_file(cyanos::delivery_pr_identity_path(&task))?;
        let ambiguous_result = Terminal::open_or_update_pr_with_program(
            ambiguous.to_string_lossy().as_ref(),
            &task,
            &selected,
            &config,
            "## Task",
        );
        assert_result_error(ambiguous_result);
        let fail_result = Terminal::open_or_update_pr_with_program(
            fail.to_string_lossy().as_ref(),
            &task,
            &selected,
            &config,
            "## Task",
        );
        assert_result_error(fail_result);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn builds_pr_body_with_runtime_evidence() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("pr-body");
        let task = task_layout(&root)?;
        let selected = outcome(
            3,
            cyanos::Evaluation::accepted(),
            score(8_500, cyanos::QualityTier::Passed),
            &task,
        );

        let body = Terminal::pr_body(&task, &selected, "## Problem\n\nDo the work.");
        let empty_structural = score(0, cyanos::QualityTier::CompileFailed);

        assert!(body.contains("<!--cyanos:task=123-->"));
        assert!(body.contains("## 1. Requirement"));
        assert!(body.contains("## 2. Implementation Summary"));
        assert!(body.contains("## 3. Architecture and Functional Impact"));
        assert!(body.contains("## 4. Verification Method"));
        assert!(body.contains("## 5. Test Results"));
        assert!(body.contains("Promoted sample 3"));
        assert!(body.contains("Composite score: 8500"));
        assert_eq!(empty_structural.tests().as_i64(), 0);
        assert!(body.contains("runs/1/samples/3/eval.json"));
        assert_eq!(
            Terminal::relative_label(&task, Path::new("/outside/file")),
            "/outside/file"
        );
        Ok(())
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "single fixture walks verifier failure modes through shared sample setup"
    )]
    fn evaluates_verifier_failure_modes() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("verifier-failures");
        let non_git = root.join("non-git");
        fs::create_dir_all(&non_git)?;
        let non_git_result = Terminal::evaluate_sample_worktree(&non_git);
        if non_git_result.is_ok() {
            return Err(std::io::Error::other("expected non-git verifier to fail").into());
        }

        let compile_fail = root.join("compile-fail");
        init_git_worktree(&compile_fail)?;
        write_cargo_project(&compile_fail, "pub fn broken() -> { 1 }\n")?;
        let compile_eval = Terminal::evaluate_sample_worktree(&compile_fail)?;
        assert!(!compile_eval.evaluation().is_accepted());
        assert_eq!(
            compile_eval.score().quality_tier(),
            cyanos::QualityTier::CompileFailed
        );
        assert!(
            compile_eval
                .evaluation()
                .to_json()
                .contains("cargo_check: failed")
        );

        let clippy_fail = root.join("clippy-fail");
        init_git_worktree(&clippy_fail)?;
        write_cargo_project(
            &clippy_fail,
            "pub fn flag() -> bool { if true { true } else { false } }\n",
        )?;
        let clippy_eval = Terminal::evaluate_sample_worktree(&clippy_fail)?;
        assert!(!clippy_eval.evaluation().is_accepted());
        assert!(
            clippy_eval
                .evaluation()
                .to_json()
                .contains("cargo_clippy: failed")
        );

        let test_fail = root.join("test-fail");
        init_git_worktree(&test_fail)?;
        write_cargo_project(
            &test_fail,
            "pub fn answer() -> u8 { 41 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn answer_is_42() {\n        assert_eq!(super::answer(), 42);\n    }\n}\n",
        )?;
        let test_eval = Terminal::evaluate_sample_worktree(&test_fail)?;
        assert!(!test_eval.evaluation().is_accepted());
        assert!(
            test_eval
                .evaluation()
                .to_json()
                .contains("cargo_test: failed")
        );

        let passing = root.join("passing");
        init_git_worktree(&passing)?;
        write_cargo_project(
            &passing,
            "pub fn answer() -> u8 { 42 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn answer_is_42() {\n        assert_eq!(super::answer(), 42);\n    }\n}\n",
        )?;
        let passing_eval =
            Terminal::evaluate_sample_worktree_with_coverage(&passing, |_| Ok(10_000))?;
        assert!(passing_eval.evaluation().is_accepted());
        assert_eq!(
            passing_eval.score().quality_tier(),
            cyanos::QualityTier::Passed
        );
        assert!(
            passing_eval
                .evaluation()
                .to_json()
                .contains("coverage: 100.00%")
        );
        assert!(passing_eval.evaluation().to_json().contains(
            "judge: agent=unconfigured model=unconfigured isolation=unconfigured verdict=pass rubric=mvp-2026-05-24"
        ));

        let judge_fail = root.join("judge-fail");
        init_git_worktree(&judge_fail)?;
        write_cargo_project(
            &judge_fail,
            "pub fn answer() -> u8 { 42 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn answer_is_42() {\n        assert_eq!(super::answer(), 42);\n    }\n}\n",
        )?;
        let judge_eval = Terminal::evaluate_sample_worktree_with_verifiers(
            &judge_fail,
            "## Problem\n\nReturn 42.",
            |_| Ok(10_000),
            |_worktree, _prompt| {
                let Some(finding) = cyanos::JudgeFinding::from_segment(
                    "category=tests,severity=major,concern=missing docs,fix=add docs,evidence=diff",
                ) else {
                    return Err(cyanos::AgentRuntimeError::new(
                        "valid judge finding fixture".to_owned(),
                    )
                    .into());
                };
                Ok(cyanos::JudgeReview::rejected("missing docs", vec![finding]))
            },
        )?;
        assert!(!judge_eval.evaluation().is_accepted());
        assert_eq!(judge_eval.score().judge().as_i64(), 5_000);
        assert!(
            judge_eval
                .evaluation()
                .to_json()
                .contains("judge finding category=tests severity=major")
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
