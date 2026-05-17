//! User-facing CLI error reporting.

use std::{fmt, io};

use crate::{
    AgentRuntimeError, CyanosCli, DependencyError, ProjectInitError, ProjectLockError,
    PromptWriteError, TaskRunError,
    cli::CliError,
    terms::{CLI_FLAG_AGENT, CLI_FLAG_MODEL, CLI_FLAG_PROJECT, CLI_FLAG_SAMPLES, CLI_FLAG_TASK},
};

const USAGE_ERROR_EXIT_CODE: u8 = 2;
const FAILURE_EXIT_CODE: u8 = 1;

/// Process exit status selected by the unified error handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitStatus {
    /// Command-line usage error.
    UsageError,
    /// Runtime or I/O failure.
    Failure,
}

impl ExitStatus {
    /// Returns the process exit code.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::UsageError => USAGE_ERROR_EXIT_CODE,
            Self::Failure => FAILURE_EXIT_CODE,
        }
    }
}

/// Application error handled at the CLI boundary.
#[derive(Debug)]
pub enum AppError {
    /// User provided invalid command-line input.
    Usage(CliError),
    /// The CLI failed while interacting with the terminal or filesystem.
    Io {
        /// User-facing action that failed.
        action: &'static str,
        /// Original I/O error.
        source: io::Error,
    },
    /// Project initialization failed.
    ProjectInit(ProjectInitError),
    /// Runtime dependency checks failed.
    Dependency(DependencyError),
    /// Task bootstrap failed.
    TaskRun(TaskRunError),
    /// Runtime prompt files could not be written.
    PromptWrite(PromptWriteError),
    /// Agent execution failed.
    AgentRuntime(AgentRuntimeError),
    /// Project lock could not be acquired.
    ProjectLock(ProjectLockError),
}

impl AppError {
    /// Creates an I/O application error.
    #[must_use]
    pub const fn io(action: &'static str, source: io::Error) -> Self {
        Self::Io { action, source }
    }

    /// Returns the process status for this error.
    #[must_use]
    pub const fn exit_status(&self) -> ExitStatus {
        match self {
            Self::Usage(_) => ExitStatus::UsageError,
            Self::Io { .. }
            | Self::ProjectInit(_)
            | Self::Dependency(_)
            | Self::TaskRun(_)
            | Self::PromptWrite(_)
            | Self::AgentRuntime(_)
            | Self::ProjectLock(_) => ExitStatus::Failure,
        }
    }

    fn user_message(&self) -> String {
        match self {
            Self::Usage(error) => error.to_string(),
            Self::Io { action, .. } => format!("failed to {action}"),
            Self::ProjectInit(error) => error.to_string(),
            Self::Dependency(error) => error.to_string(),
            Self::TaskRun(error) => error.to_string(),
            Self::PromptWrite(error) => error.to_string(),
            Self::AgentRuntime(error) => error.to_string(),
            Self::ProjectLock(error) => error.to_string(),
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "centralized user-facing hint mapping is easier to audit than split tables"
    )]
    fn hint(&self) -> &'static str {
        match self {
            Self::Usage(CliError::MissingCommand) => "use 'cyanos init' or 'cyanos run'",
            Self::Usage(CliError::MissingProjectId) => {
                "provide a repository locator, for example '--project owner/repo'"
            }
            Self::Usage(CliError::MissingTaskId) => "provide a task id, for example '--task 123'",
            Self::Usage(CliError::MissingValue(CLI_FLAG_PROJECT)) => {
                "provide a repository locator after '--project'"
            }
            Self::Usage(CliError::MissingValue(CLI_FLAG_TASK)) => {
                "provide a path-safe task id after '--task'"
            }
            Self::Usage(CliError::MissingValue(CLI_FLAG_AGENT)) => {
                "provide an agent name, for example '--agent codex'"
            }
            Self::Usage(CliError::MissingValue(CLI_FLAG_MODEL)) => {
                "provide a model name, for example '--model gpt-5.5'"
            }
            Self::Usage(CliError::MissingValue(CLI_FLAG_SAMPLES)) => {
                "provide a positive integer after '--samples'"
            }
            Self::Usage(CliError::MissingValue(_)) => "provide a value after the option",
            Self::Usage(CliError::UnknownCommand(_)) => "supported commands: init, run",
            Self::Usage(CliError::UnknownAgent(_)) => "supported agents: claude, codex",
            Self::Usage(CliError::UnknownFlag(_)) => {
                "run 'cyanos init' or 'cyanos run' with supported flags"
            }
            Self::Usage(CliError::UnexpectedArgument(_)) => "use flags instead of positional input",
            Self::Usage(CliError::InvalidProjectId(_)) => {
                "use owner/repo for github.com, or a full http(s) repository URL for GitHub Enterprise"
            }
            Self::Usage(CliError::InvalidTaskId(_)) => {
                "task ids may contain only ASCII letters, digits, '-' and '_'"
            }
            Self::Usage(CliError::InvalidSampleCount(_)) => {
                "sample count must be a positive integer"
            }
            Self::ProjectInit(ProjectInitError::ProjectAlreadyExists(_)) => {
                "remove the existing managed project if you want to reinitialize this repository"
            }
            Self::ProjectInit(
                ProjectInitError::SourceMissing(_) | ProjectInitError::SourceNotDirectory(_),
            ) => {
                "run 'cyanos init --project <repo-locator>' from an existing repository or workspace"
            }
            Self::ProjectInit(ProjectInitError::InvalidProjectLocator(_)) => {
                "use '--project owner/repo' for github.com or a full repository URL for GitHub Enterprise"
            }
            Self::ProjectInit(ProjectInitError::OriginRemoteMismatch { .. }) => {
                "run init from the matching repository checkout or pass the repository that matches origin"
            }
            Self::ProjectInit(ProjectInitError::HomeUnavailable) => {
                "set HOME or pass a supported runtime home in a future configuration option"
            }
            Self::ProjectInit(_) => "check filesystem permissions and git availability",
            Self::Dependency(DependencyError::CommandSpawn { .. }) => {
                "install the missing executable and ensure it is available on PATH"
            }
            Self::Dependency(DependencyError::CommandFailed { .. }) => {
                "authenticate gh and the selected agent CLI before running cyanos"
            }
            Self::TaskRun(TaskRunError::ProjectMissing(_))
            | Self::ProjectLock(ProjectLockError::ProjectMissing(_)) => {
                "run 'cyanos init --project <repo-locator>' before running this project"
            }
            Self::TaskRun(TaskRunError::MissingTaskSource(_)) => {
                "add .cyanos/README.md to the origin repository with task retrieval instructions"
            }
            Self::TaskRun(TaskRunError::MissingTaskId(_)) => {
                "add a project-owned task id such as 'Task ID: 123' before running evolution"
            }
            Self::TaskRun(TaskRunError::MissingProjectConfig(_)) => {
                "run 'cyanos init --project <repo-locator>' again to create managed project config"
            }
            Self::TaskRun(TaskRunError::UnknownRepository(_)) => {
                "run 'cyanos init --project <repo-locator>' from the target repository"
            }
            Self::TaskRun(
                TaskRunError::GitHubTaskFailed { .. } | TaskRunError::GitHubCommentFailed { .. },
            ) => "check gh authentication and repository permissions, then retry",
            Self::TaskRun(TaskRunError::RuntimeRepositoryInvalid { .. }) => {
                "configure a runtime repository that is separate from the target source repository"
            }
            Self::TaskRun(TaskRunError::RuntimeRepositoryFailed { .. }) => {
                "check runtime repository permissions and gh authentication, then retry"
            }
            Self::TaskRun(TaskRunError::PreflightFailed { .. }) => {
                "fix the failed preflight check and retry"
            }
            Self::TaskRun(TaskRunError::GitSpawn { .. } | TaskRunError::GitFailed { .. }) => {
                "check the managed origin repository and git worktree state"
            }
            Self::TaskRun(_) => "check the managed project directory and task permissions",
            Self::PromptWrite(_) => "check task worktree permissions and available disk space",
            Self::AgentRuntime(_) => {
                "inspect the selected agent CLI output and retry after fixing the reported issue"
            }
            Self::ProjectLock(ProjectLockError::AlreadyLocked(_)) => {
                "wait for the existing cyanos process to finish or remove a stale lock after inspection"
            }
            Self::ProjectLock(_) => "check managed project permissions",
            Self::Io { .. } => "retry the command after checking terminal or filesystem access",
        }
    }

    fn failed_check(&self) -> Option<String> {
        match self {
            Self::Dependency(DependencyError::CommandSpawn { program, .. }) => {
                Some(format!("{program} available on PATH"))
            }
            Self::Dependency(DependencyError::CommandFailed { command, .. }) => {
                Some(command.join(" "))
            }
            Self::TaskRun(TaskRunError::PreflightFailed { check, .. }) => Some(check.clone()),
            Self::Usage(_)
            | Self::Io { .. }
            | Self::ProjectInit(_)
            | Self::TaskRun(_)
            | Self::PromptWrite(_)
            | Self::AgentRuntime(_)
            | Self::ProjectLock(_) => None,
        }
    }
}

impl From<CliError> for AppError {
    fn from(error: CliError) -> Self {
        Self::Usage(error)
    }
}

impl From<ProjectInitError> for AppError {
    fn from(error: ProjectInitError) -> Self {
        Self::ProjectInit(error)
    }
}

impl From<DependencyError> for AppError {
    fn from(error: DependencyError) -> Self {
        Self::Dependency(error)
    }
}

impl From<TaskRunError> for AppError {
    fn from(error: TaskRunError) -> Self {
        Self::TaskRun(error)
    }
}

impl From<PromptWriteError> for AppError {
    fn from(error: PromptWriteError) -> Self {
        Self::PromptWrite(error)
    }
}

impl From<AgentRuntimeError> for AppError {
    fn from(error: AgentRuntimeError) -> Self {
        Self::AgentRuntime(error)
    }
}

impl From<ProjectLockError> for AppError {
    fn from(error: ProjectLockError) -> Self {
        Self::ProjectLock(error)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(error) => write!(f, "{error}"),
            Self::Io { action, source } => write!(f, "failed to {action}: {source}"),
            Self::ProjectInit(error) => write!(f, "{error}"),
            Self::Dependency(error) => write!(f, "{error}"),
            Self::TaskRun(error) => write!(f, "{error}"),
            Self::PromptWrite(error) => write!(f, "{error}"),
            Self::AgentRuntime(error) => write!(f, "{error}"),
            Self::ProjectLock(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Usage(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::ProjectInit(error) => Some(error),
            Self::Dependency(error) => Some(error),
            Self::TaskRun(error) => Some(error),
            Self::PromptWrite(error) => Some(error),
            Self::AgentRuntime(error) => Some(error),
            Self::ProjectLock(error) => Some(error),
        }
    }
}

/// Rendered CLI error report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorReport {
    exit_status: ExitStatus,
    body: String,
}

impl ErrorReport {
    /// Creates a rendered error report.
    #[must_use]
    pub fn new(exit_status: ExitStatus, body: String) -> Self {
        Self { exit_status, body }
    }

    /// Returns the selected exit status.
    #[must_use]
    pub const fn exit_status(&self) -> ExitStatus {
        self.exit_status
    }

    /// Returns the user-facing report body.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

/// Central CLI error reporter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ErrorReporter {
    product_name: &'static str,
}

impl Default for ErrorReporter {
    fn default() -> Self {
        Self {
            product_name: CyanosCli::PRODUCT_NAME,
        }
    }
}

impl ErrorReporter {
    /// Creates an error reporter.
    #[must_use]
    pub const fn new(product_name: &'static str) -> Self {
        Self { product_name }
    }

    /// Renders a user-friendly error report.
    #[must_use]
    pub fn render(&self, error: &AppError) -> ErrorReport {
        let mut body = format!("{}: error: {}", self.product_name, error.user_message());

        if let Some(check) = error.failed_check() {
            body.push('\n');
            body.push_str("check: ❌ ");
            body.push_str(&check);
        }
        body.push('\n');
        body.push_str("hint: ");
        body.push_str(error.hint());

        ErrorReport::new(error.exit_status(), body)
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io};

    use crate::{
        AgentRuntimeError, AppError, CliError, CyanosHome, DependencyError, ErrorReporter,
        ExitStatus, InnerPrompt, OuterPrompt, ProjectId, ProjectInitError, ProjectLockError,
        PromptSet, RuntimePromptWriter, TaskId, TaskRunError,
    };

    #[test]
    fn renders_usage_error_with_actionable_hint() {
        let error = AppError::from(CliError::UnknownAgent("unknown".to_owned()));
        let report = ErrorReporter::new("cyanos").render(&error);

        assert_eq!(report.exit_status(), ExitStatus::UsageError);
        assert_eq!(
            report.body(),
            "cyanos: error: unknown agent: unknown\nhint: supported agents: claude, codex"
        );
    }

    #[test]
    fn renders_io_error_without_losing_source() {
        let error = AppError::io("write CLI output", io::Error::other("closed pipe"));
        let report = ErrorReporter::new("cyanos").render(&error);

        assert_eq!(report.exit_status(), ExitStatus::Failure);
        assert_eq!(
            report.body(),
            "cyanos: error: failed to write CLI output\nhint: retry the command after checking terminal or filesystem access"
        );
        assert_eq!(error.to_string(), "failed to write CLI output: closed pipe");
    }

    #[test]
    fn renders_dependency_error_with_actionable_hint() {
        let error = AppError::from(DependencyError::CommandFailed {
            command: vec!["gh".to_owned(), "auth".to_owned(), "status".to_owned()],
            code: Some(1),
        });
        let report = ErrorReporter::new("cyanos").render(&error);

        assert_eq!(report.exit_status(), ExitStatus::Failure);
        assert_eq!(
            report.body(),
            "cyanos: error: dependency check `gh auth status` failed with exit code Some(1)\ncheck: ❌ gh auth status\nhint: authenticate gh and the selected agent CLI before running cyanos"
        );
    }

    #[test]
    fn renders_task_run_error_with_actionable_hint() {
        let error = AppError::from(TaskRunError::MissingTaskSource("/tmp/task.md".into()));
        let report = ErrorReporter::new("cyanos").render(&error);

        assert_eq!(report.exit_status(), ExitStatus::Failure);
        assert_eq!(
            report.body(),
            "cyanos: error: task source README does not exist at /tmp/task.md\nhint: add .cyanos/README.md to the origin repository with task retrieval instructions"
        );
    }

    #[test]
    fn renders_prompt_write_error_with_actionable_hint() -> Result<(), Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!("cyanos-error-prompt-{}", std::process::id()));
        let task = CyanosHome::new(root.join("home"))
            .project(ProjectId::new("demo")?)
            .task(TaskId::new("123".to_owned())?);
        let prompts = PromptSet::new(
            OuterPrompt::new("outer".to_owned()),
            InnerPrompt::new("inner".to_owned()),
        );
        let result = RuntimePromptWriter::write_initial(&task, &prompts);
        let Err(error) = result else {
            return Err(io::Error::other("expected prompt write error").into());
        };
        let error = AppError::from(error);
        assert!(error.to_string().contains("failed to write runtime"));
        let report = ErrorReporter::new("cyanos").render(&error);

        assert_eq!(report.exit_status(), ExitStatus::Failure);
        assert!(
            report
                .body()
                .contains("hint: check task worktree permissions and available disk space")
        );

        Ok(())
    }

    #[test]
    fn renders_agent_runtime_error_with_actionable_hint() {
        let error = AppError::from(AgentRuntimeError::new("agent failed".to_owned()));
        let report = ErrorReporter::new("cyanos").render(&error);

        assert_eq!(report.exit_status(), ExitStatus::Failure);
        assert_eq!(
            report.body(),
            "cyanos: error: agent failed\nhint: inspect the selected agent CLI output and retry after fixing the reported issue"
        );
    }

    #[test]
    fn renders_project_lock_error_with_actionable_hint() {
        let error = AppError::from(ProjectLockError::AlreadyLocked("/tmp/cyanos.lock".into()));
        let report = ErrorReporter::new("cyanos").render(&error);

        assert_eq!(report.exit_status(), ExitStatus::Failure);
        assert_eq!(
            report.body(),
            "cyanos: error: project is already locked at /tmp/cyanos.lock\nhint: wait for the existing cyanos process to finish or remove a stale lock after inspection"
        );
    }

    #[test]
    fn renders_runtime_failure_hints() {
        let reporter = ErrorReporter::new("cyanos");
        let project_exists = AppError::from(ProjectInitError::ProjectAlreadyExists(
            "/tmp/project".into(),
        ));
        let source_missing = AppError::from(ProjectInitError::SourceMissing("/tmp/source".into()));
        let home_missing = AppError::from(ProjectInitError::HomeUnavailable);
        let dependency_spawn = AppError::from(DependencyError::CommandSpawn {
            program: "codex".to_owned(),
            source: io::Error::new(io::ErrorKind::NotFound, "missing"),
        });
        let task_git = AppError::from(TaskRunError::GitFailed {
            cwd: "/tmp/repo".into(),
            args: vec!["worktree".to_owned()],
            code: Some(1),
        });
        let task_io = AppError::from(TaskRunError::Io {
            action: "write",
            path: "/tmp/task".into(),
            source: io::Error::other("denied"),
        });
        let missing_task_id = AppError::from(TaskRunError::MissingTaskId("/tmp/task.md".into()));
        let lock_missing = AppError::from(ProjectLockError::ProjectMissing("/tmp/project".into()));
        let lock_io = AppError::from(ProjectLockError::Io {
            action: "create",
            path: "/tmp/lock".into(),
            source: io::Error::other("denied"),
        });

        assert!(
            reporter
                .render(&project_exists)
                .body()
                .contains("existing managed project")
        );
        assert!(
            reporter
                .render(&source_missing)
                .body()
                .contains("existing repository")
        );
        assert!(reporter.render(&home_missing).body().contains("set HOME"));
        assert!(
            reporter
                .render(&dependency_spawn)
                .body()
                .contains("install the missing executable")
        );
        assert!(
            reporter
                .render(&dependency_spawn)
                .body()
                .contains("check: ❌ codex available on PATH")
        );
        assert!(
            reporter
                .render(&task_git)
                .body()
                .contains("git worktree state")
        );
        assert!(
            reporter
                .render(&task_io)
                .body()
                .contains("task permissions")
        );
        assert!(
            reporter
                .render(&missing_task_id)
                .body()
                .contains("project-owned task id")
        );
        assert!(
            reporter
                .render(&lock_missing)
                .body()
                .contains("cyanos init")
        );
        assert!(
            reporter
                .render(&lock_io)
                .body()
                .contains("managed project permissions")
        );
    }

    #[test]
    fn renders_project_locator_init_hints() {
        let reporter = ErrorReporter::new("cyanos");
        let invalid_locator =
            AppError::from(ProjectInitError::InvalidProjectLocator("demo".to_owned()));
        let origin_mismatch = AppError::from(ProjectInitError::OriginRemoteMismatch {
            expected: "owner/repo".to_owned(),
            actual: "other/repo".to_owned(),
        });

        assert!(
            reporter
                .render(&invalid_locator)
                .body()
                .contains("full repository URL")
        );
        assert!(
            reporter
                .render(&origin_mismatch)
                .body()
                .contains("matching repository checkout")
        );
    }

    #[test]
    fn renders_all_usage_hints() {
        let reporter = ErrorReporter::new("cyanos");
        let cases = [
            (
                CliError::MissingCommand,
                "cyanos: error: missing command\nhint: use 'cyanos init' or 'cyanos run'",
            ),
            (
                CliError::MissingProjectId,
                "cyanos: error: missing required --project\nhint: provide a repository locator, for example '--project owner/repo'",
            ),
            (
                CliError::MissingTaskId,
                "cyanos: error: missing required --task\nhint: provide a task id, for example '--task 123'",
            ),
            (
                CliError::MissingValue("--project"),
                "cyanos: error: missing value for --project\nhint: provide a repository locator after '--project'",
            ),
            (
                CliError::MissingValue("--task"),
                "cyanos: error: missing value for --task\nhint: provide a path-safe task id after '--task'",
            ),
            (
                CliError::MissingValue("--agent"),
                "cyanos: error: missing value for --agent\nhint: provide an agent name, for example '--agent codex'",
            ),
            (
                CliError::MissingValue("--model"),
                "cyanos: error: missing value for --model\nhint: provide a model name, for example '--model gpt-5.5'",
            ),
            (
                CliError::MissingValue("--samples"),
                "cyanos: error: missing value for --samples\nhint: provide a positive integer after '--samples'",
            ),
            (
                CliError::MissingValue("--other"),
                "cyanos: error: missing value for --other\nhint: provide a value after the option",
            ),
            (
                CliError::InvalidSampleCount("0".to_owned()),
                "cyanos: error: invalid --samples value: 0; use a positive integer\nhint: sample count must be a positive integer",
            ),
            (
                CliError::UnknownCommand("status".to_owned()),
                "cyanos: error: unknown command: status\nhint: supported commands: init, run",
            ),
            (
                CliError::UnknownFlag("--bad".to_owned()),
                "cyanos: error: unknown flag: --bad\nhint: run 'cyanos init' or 'cyanos run' with supported flags",
            ),
            (
                CliError::UnexpectedArgument("x".to_owned()),
                "cyanos: error: unexpected argument: x\nhint: use flags instead of positional input",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(reporter.render(&AppError::from(error)).body(), expected);
        }
    }

    #[test]
    fn exposes_exit_codes_and_error_sources() {
        let usage = AppError::from(CliError::MissingCommand);
        let io_error = AppError::io("read config", io::Error::other("missing"));

        assert_eq!(ExitStatus::UsageError.code(), 2);
        assert_eq!(ExitStatus::Failure.code(), 1);
        assert_eq!(usage.exit_status(), ExitStatus::UsageError);
        assert_eq!(io_error.exit_status(), ExitStatus::Failure);
        assert!(usage.source().is_some());
        assert!(io_error.source().is_some());
        let dependency = AppError::from(DependencyError::CommandSpawn {
            program: "codex".to_owned(),
            source: io::Error::new(io::ErrorKind::NotFound, "missing"),
        });
        assert!(dependency.source().is_some());
        let task = AppError::from(TaskRunError::ProjectMissing("/tmp/project".into()));
        assert!(task.source().is_some());
        let agent = AppError::from(AgentRuntimeError::new("failed".to_owned()));
        assert!(agent.source().is_some());
        let lock = AppError::from(ProjectLockError::ProjectMissing("/tmp/project".into()));
        assert!(lock.source().is_some());
    }

    #[test]
    fn displays_all_app_error_variants() {
        let variants = [
            AppError::from(CliError::MissingCommand),
            AppError::io("read config", io::Error::other("missing")),
            AppError::from(ProjectInitError::HomeUnavailable),
            AppError::from(DependencyError::CommandFailed {
                command: vec!["gh".to_owned()],
                code: Some(1),
            }),
            AppError::from(TaskRunError::ProjectMissing("/tmp/project".into())),
            AppError::from(AgentRuntimeError::new("agent failed".to_owned())),
            AppError::from(ProjectLockError::ProjectMissing("/tmp/project".into())),
        ];

        for error in variants {
            assert!(!error.to_string().is_empty());
        }
    }
}
