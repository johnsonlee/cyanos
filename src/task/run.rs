//! Task bootstrap for `cyanos run`.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::terms::{
    AUTO_ARG, CYANOS_DIR, FETCH_ARG, GIT_PROGRAM, HEAD_ARG, ORIGIN_REMOTE, PRUNE_FLAG, README_FILE,
    REMOTE_ARG, REMOTE_HEAD_REF, SET_HEAD_ARG, WORKTREE_ARG,
};
use crate::{
    CyanosHome, EvaluationSnapshot, IdentifierError, ProjectId, ProjectLayout, TaskId, TaskLayout,
    TaskResultSnapshot, process, read_best_record,
};

const BEST_COMMIT_AVAILABILITY_CHECK: &str = "best commit availability";
const BEST_COMMIT_TYPE_SUFFIX: &str = "^{commit}";
const DISCARD_INCOMPLETE_SAMPLE_RUNTIME_ACTION: &str =
    "discard incomplete sample runtime directory";
const FORCE_FLAG: &str = "--force";
const GIT_REMOTE_GET_URL_ARG: &str = "get-url";
const GIT_WORKTREE_REMOVE_ARG: &str = "remove";
const READ_BEST_METADATA_ACTION: &str = "read global best metadata";
const RESTORE_BEST_COMMIT_ACTION: &str =
    "fetch or restore the promoted best commit before rerunning Cyanos";
const REV_PARSE_ARG: &str = "rev-parse";
const VERIFY_ARG: &str = "--verify";
const TASK_ID_HEADERS: [&str; 4] = ["Task ID:", "Task-ID:", "Issue ID:", "Issue:"];

/// Request to bootstrap the next task for a managed project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRunRequest {
    home: CyanosHome,
    project_id: ProjectId,
    task_id: TaskId,
}

impl TaskRunRequest {
    /// Creates a task run request.
    #[must_use]
    pub const fn new(home: CyanosHome, project_id: ProjectId, task_id: TaskId) -> Self {
        Self {
            home,
            project_id,
            task_id,
        }
    }

    /// Returns the managed project layout.
    #[must_use]
    pub fn project(&self) -> ProjectLayout {
        self.home.project(self.project_id.clone())
    }

    /// Returns the project id.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the selected task id.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }
}

/// Result of bootstrapping one task runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRunReport {
    task: TaskLayout,
    source_readme: PathBuf,
}

impl TaskRunReport {
    /// Creates a task run report.
    #[must_use]
    pub fn new(task: TaskLayout, source_readme: PathBuf) -> Self {
        Self {
            task,
            source_readme,
        }
    }

    /// Returns the task layout.
    #[must_use]
    pub const fn task(&self) -> &TaskLayout {
        &self.task
    }

    /// Returns the origin task-source README path.
    #[must_use]
    pub fn source_readme(&self) -> &Path {
        &self.source_readme
    }
}

/// Git runner used by task bootstrap.
pub trait TaskGitRunner {
    /// Runs `git` with args in the provided directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be launched or exits
    /// unsuccessfully.
    fn run(&self, cwd: &Path, args: &[&str]) -> Result<(), TaskRunError>;

    /// Returns whether a `git` command exits successfully.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be launched.
    fn succeeds(&self, cwd: &Path, args: &[&str]) -> Result<bool, TaskRunError>;
}

impl<T> TaskGitRunner for &T
where
    T: TaskGitRunner,
{
    fn run(&self, cwd: &Path, args: &[&str]) -> Result<(), TaskRunError> {
        (*self).run(cwd, args)
    }

    fn succeeds(&self, cwd: &Path, args: &[&str]) -> Result<bool, TaskRunError> {
        (*self).succeeds(cwd, args)
    }
}

/// System `git` runner for task bootstrap.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemTaskGitRunner;

impl TaskGitRunner for SystemTaskGitRunner {
    fn run(&self, cwd: &Path, args: &[&str]) -> Result<(), TaskRunError> {
        let output =
            process::output(GIT_PROGRAM, cwd, args).map_err(|source| TaskRunError::GitSpawn {
                cwd: cwd.to_path_buf(),
                source,
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(TaskRunError::GitFailed {
                cwd: cwd.to_path_buf(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                code: output.status.code(),
            })
        }
    }

    fn succeeds(&self, cwd: &Path, args: &[&str]) -> Result<bool, TaskRunError> {
        process::output(GIT_PROGRAM, cwd, args)
            .map(|output| output.status.success())
            .map_err(|source| TaskRunError::GitSpawn {
                cwd: cwd.to_path_buf(),
                source,
            })
    }
}

/// Bootstraps the next task runtime.
#[derive(Clone, Debug)]
pub struct TaskRunner<G> {
    git: G,
}

impl<G> TaskRunner<G>
where
    G: TaskGitRunner,
{
    /// Creates a task runner.
    #[must_use]
    pub const fn new(git: G) -> Self {
        Self { git }
    }

    /// Creates the task runtime directory, sample execution worktrees, and task README.
    ///
    /// # Errors
    ///
    /// Returns an error when the project is missing, the task source is
    /// missing, git worktree creation fails, or task files cannot be written.
    pub fn bootstrap(&self, request: &TaskRunRequest) -> Result<TaskRunReport, TaskRunError> {
        self.bootstrap_samples(request, 1)
    }

    /// Creates the task runtime with the requested sample worktrees.
    ///
    /// # Errors
    ///
    /// Returns an error when the project is missing, the task source is
    /// missing, git worktree creation fails, or task files cannot be written.
    pub fn bootstrap_samples(
        &self,
        request: &TaskRunRequest,
        sample_count: usize,
    ) -> Result<TaskRunReport, TaskRunError> {
        let project = request.project();
        if !project.origin().is_dir() {
            return Err(TaskRunError::ProjectMissing(project.root()));
        }

        let base_ref = self.fetch_remote_if_present(&project)?;
        let source = TaskSource::load(&project.origin())?;
        self.bootstrap_with_source(request, source, sample_count, base_ref)
    }

    /// Creates a task runtime from an explicit task brief.
    ///
    /// # Errors
    ///
    /// Returns an error when the project is missing, git worktree creation
    /// fails, or task files cannot be written.
    pub fn bootstrap_task(
        &self,
        request: &TaskRunRequest,
        source_label: impl Into<PathBuf>,
        content: String,
        sample_count: usize,
    ) -> Result<TaskRunReport, TaskRunError> {
        let project = request.project();
        if !project.origin().is_dir() {
            return Err(TaskRunError::ProjectMissing(project.root()));
        }

        let base_ref = self.fetch_remote_if_present(&project)?;
        let source = TaskSource::new(request.task_id().clone(), source_label.into(), content);
        self.bootstrap_with_source(request, source, sample_count, base_ref)
    }

    fn bootstrap_with_source(
        &self,
        request: &TaskRunRequest,
        source: TaskSource,
        sample_count: usize,
        base_ref: &str,
    ) -> Result<TaskRunReport, TaskRunError> {
        let project = request.project();
        let task = project.task(source.task_id.clone());

        Self::create_dir(&project.tasks(), "create task directory")?;
        Self::create_dir(&task.root(), "create task runtime directory")?;
        for sample_id in 1..=sample_count {
            Self::create_dir(
                &task.sample_run_root(sample_id, 1),
                "create sample runtime directory",
            )?;
            self.add_sample_worktree(&project, &task, (sample_id, 1), base_ref)?;
        }
        Self::write_task_readme(request.project_id(), &task, &source)?;

        Ok(TaskRunReport::new(task, source.path))
    }

    /// Creates sample worktrees for an additional outer run.
    ///
    /// # Errors
    ///
    /// Returns an error when the project is missing or git worktree creation
    /// fails.
    pub fn bootstrap_sample_run(
        &self,
        request: &TaskRunRequest,
        run_index: usize,
        sample_count: usize,
    ) -> Result<(), TaskRunError> {
        let project = request.project();
        if !project.origin().is_dir() {
            return Err(TaskRunError::ProjectMissing(project.root()));
        }

        let task = project.task(request.task_id().clone());
        Self::create_dir(&task.run_root(run_index), "create run runtime directory")?;
        let base_ref = self.sample_run_base_ref(&project, &task, run_index)?;
        for sample_id in 1..=sample_count {
            Self::create_dir(
                &task.sample_run_root(sample_id, run_index),
                "create sample runtime directory",
            )?;
            self.add_sample_worktree(&project, &task, (sample_id, run_index), &base_ref)?;
        }

        Ok(())
    }

    fn sample_run_base_ref(
        &self,
        project: &ProjectLayout,
        task: &TaskLayout,
        run_index: usize,
    ) -> Result<String, TaskRunError> {
        if run_index > 1 && task.root().is_dir() {
            let evolution_branch = task.evolution_branch_name();
            if self.git.succeeds(
                &project.origin(),
                &[REV_PARSE_ARG, VERIFY_ARG, evolution_branch.as_str()],
            )? {
                return Ok(evolution_branch);
            }

            let fallback_ref = self.fetch_remote_if_present(project)?;
            if let Some(best_ref) = self.best_resume_ref(project, task)? {
                return Ok(best_ref);
            }
            return Ok(fallback_ref.to_owned());
        }

        self.fetch_remote_if_present(project).map(str::to_owned)
    }

    fn best_resume_ref(
        &self,
        project: &ProjectLayout,
        task: &TaskLayout,
    ) -> Result<Option<String>, TaskRunError> {
        let record = match read_best_record(&task.best()) {
            Ok(record) => record,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(TaskRunError::Io {
                    action: READ_BEST_METADATA_ACTION,
                    path: task.best(),
                    source,
                });
            }
        };

        for candidate in [record.cyanos_commit(), record.target_commit()] {
            if candidate.trim().is_empty() {
                continue;
            }
            let commit_ref = format!("{candidate}{BEST_COMMIT_TYPE_SUFFIX}");
            if self.git.succeeds(
                &project.origin(),
                &[REV_PARSE_ARG, VERIFY_ARG, commit_ref.as_str()],
            )? {
                return Ok(Some(candidate.to_owned()));
            }
        }

        Err(TaskRunError::PreflightFailed {
            check: format!(
                "{BEST_COMMIT_AVAILABILITY_CHECK} for task {}",
                task.task_id().as_str()
            ),
            next_action: RESTORE_BEST_COMMIT_ACTION.to_owned(),
        })
    }

    fn fetch_remote_if_present(
        &self,
        project: &ProjectLayout,
    ) -> Result<&'static str, TaskRunError> {
        if self.git.succeeds(
            &project.origin(),
            &[REMOTE_ARG, GIT_REMOTE_GET_URL_ARG, ORIGIN_REMOTE],
        )? {
            self.git
                .run(&project.origin(), &[FETCH_ARG, "--all", PRUNE_FLAG])?;
            self.git.run(
                &project.origin(),
                &[REMOTE_ARG, SET_HEAD_ARG, ORIGIN_REMOTE, AUTO_ARG],
            )?;
            return Ok(REMOTE_HEAD_REF);
        }

        Ok(HEAD_ARG)
    }

    fn add_sample_worktree(
        &self,
        project: &ProjectLayout,
        task: &TaskLayout,
        sample: (usize, usize),
        base_ref: &str,
    ) -> Result<(), TaskRunError> {
        let (sample_id, run_index) = sample;
        if task.sample_run_worktree(sample_id, run_index).is_dir() {
            if sample_run_has_completed_evidence(task, sample_id, run_index) {
                return Ok(());
            }
            self.discard_incomplete_sample_run(project, task, sample)?;
        }
        let sample_worktree = task
            .sample_run_worktree(sample_id, run_index)
            .to_string_lossy()
            .to_string();
        self.git.run(
            &project.origin(),
            &[
                WORKTREE_ARG,
                "add",
                "--detach",
                sample_worktree.as_str(),
                base_ref,
            ],
        )
    }

    fn discard_incomplete_sample_run(
        &self,
        project: &ProjectLayout,
        task: &TaskLayout,
        sample: (usize, usize),
    ) -> Result<(), TaskRunError> {
        let (sample_id, run_index) = sample;
        let sample_worktree = task.sample_run_worktree(sample_id, run_index);
        let sample_worktree_arg = sample_worktree.to_string_lossy().to_string();
        if sample_worktree.is_dir() {
            self.git.run(
                &project.origin(),
                &[
                    WORKTREE_ARG,
                    GIT_WORKTREE_REMOVE_ARG,
                    FORCE_FLAG,
                    sample_worktree_arg.as_str(),
                ],
            )?;
        }
        let sample_root = task.sample_run_root(sample_id, run_index);
        if sample_root.exists() {
            fs::remove_dir_all(&sample_root).map_err(|source| TaskRunError::Io {
                action: DISCARD_INCOMPLETE_SAMPLE_RUNTIME_ACTION,
                path: sample_root.clone(),
                source,
            })?;
        }
        Self::create_dir(&sample_root, "create sample runtime directory")
    }

    fn create_dir(path: &Path, action: &'static str) -> Result<(), TaskRunError> {
        fs::create_dir_all(path).map_err(|source| TaskRunError::Io {
            action,
            path: path.to_path_buf(),
            source,
        })
    }

    fn write_task_readme(
        project_id: &ProjectId,
        task: &TaskLayout,
        source: &TaskSource,
    ) -> Result<(), TaskRunError> {
        let body = TaskReadme::render(project_id, task, source);
        fs::write(task.readme(), body).map_err(|source| TaskRunError::Io {
            action: "write task README",
            path: task.readme(),
            source,
        })
    }
}

fn sample_run_has_completed_evidence(
    task: &TaskLayout,
    sample_id: usize,
    run_index: usize,
) -> bool {
    let eval_path = task.sample_eval(sample_id, run_index);
    if !eval_path.is_file()
        || !task.sample_summary(sample_id, run_index).is_file()
        || !task.sample_patch(sample_id, run_index).is_file()
    {
        return false;
    }

    let Ok(eval_content) = fs::read_to_string(&eval_path) else {
        return false;
    };
    if EvaluationSnapshot::parse(&eval_content).is_err() {
        return false;
    }

    let Ok(result_content) = fs::read_to_string(task.result()) else {
        return false;
    };
    let Ok(result) = TaskResultSnapshot::parse(&result_content) else {
        return false;
    };
    let eval_label = eval_path.strip_prefix(task.root()).map_or_else(
        |_| eval_path.to_string_lossy().to_string(),
        |path| path.to_string_lossy().to_string(),
    );

    result.has_selected_sample_eval(run_index, sample_id, &eval_label)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TaskSource {
    path: PathBuf,
    task_id: TaskId,
    content: String,
}

impl TaskSource {
    fn new(task_id: TaskId, path: PathBuf, content: String) -> Self {
        Self {
            path,
            task_id,
            content,
        }
    }

    fn load(origin: &Path) -> Result<Self, TaskRunError> {
        let path = origin.join(CYANOS_DIR).join(README_FILE);
        let content = fs::read_to_string(&path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                TaskRunError::MissingTaskSource(path.clone())
            } else {
                TaskRunError::Io {
                    action: "read task source README",
                    path: path.clone(),
                    source,
                }
            }
        })?;

        let task_id = Self::parse_task_id(&path, &content)?;

        Ok(Self::new(task_id, path, content))
    }

    fn parse_task_id(path: &Path, content: &str) -> Result<TaskId, TaskRunError> {
        for line in content.lines().map(str::trim) {
            for header in TASK_ID_HEADERS {
                if let Some(value) = line.strip_prefix(header) {
                    let value = value.trim().trim_start_matches('#').trim();
                    return TaskId::new(value.to_owned()).map_err(TaskRunError::InvalidTaskId);
                }
            }
        }

        Err(TaskRunError::MissingTaskId(path.to_path_buf()))
    }
}

struct TaskReadme;

impl TaskReadme {
    fn render(project_id: &ProjectId, task: &TaskLayout, source: &TaskSource) -> String {
        format!(
            "# Cyanos Task {}\n\nProject: {}\nEvolution Branch: {}\nPR Branch: {}\nBest: {}\nSource: {}\n\n## Task Source\n\n{}",
            task.task_id().as_str(),
            project_id.as_str(),
            task.evolution_branch_name(),
            task.pr_branch_name(),
            task.best().display(),
            source.path.display(),
            source.content
        )
    }
}

/// Error returned by task bootstrap.
#[derive(Debug)]
pub enum TaskRunError {
    /// The managed project does not exist.
    ProjectMissing(PathBuf),
    /// The origin `.cyanos/README.md` task source does not exist.
    MissingTaskSource(PathBuf),
    /// The task source does not define a project-owned task id.
    MissingTaskId(PathBuf),
    /// A project-provided task id was invalid.
    InvalidTaskId(IdentifierError),
    /// The managed project has no config file.
    MissingProjectConfig(PathBuf),
    /// The managed project config does not resolve a GitHub repository.
    UnknownRepository(PathBuf),
    /// The selected GitHub task could not be loaded.
    GitHubTaskFailed {
        /// GitHub repository.
        repo: String,
        /// Task id.
        task_id: String,
        /// Process exit code.
        code: Option<i32>,
    },
    /// Blocker feedback could not be posted to GitHub.
    GitHubCommentFailed {
        /// GitHub repository.
        repo: String,
        /// Task id.
        task_id: String,
        /// Process exit code.
        code: Option<i32>,
    },
    /// The resolved runtime repository is invalid or unsafe.
    RuntimeRepositoryInvalid {
        /// Target source repository.
        source_repo: String,
        /// Runtime repository.
        runtime_repo: String,
    },
    /// Runtime repository preflight or publishing failed.
    RuntimeRepositoryFailed {
        /// Runtime repository.
        runtime_repo: String,
        /// Failed action.
        action: &'static str,
        /// Process exit code.
        code: Option<i32>,
    },
    /// A preflight prerequisite failed before coding started.
    PreflightFailed {
        /// Check item that failed.
        check: String,
        /// Suggested next action.
        next_action: String,
    },
    /// Filesystem operation failed.
    Io {
        /// Action being performed.
        action: &'static str,
        /// Path being accessed.
        path: PathBuf,
        /// Original I/O error.
        source: io::Error,
    },
    /// Git could not be launched.
    GitSpawn {
        /// Working directory.
        cwd: PathBuf,
        /// Original I/O error.
        source: io::Error,
    },
    /// Git exited unsuccessfully.
    GitFailed {
        /// Working directory.
        cwd: PathBuf,
        /// Git arguments.
        args: Vec<String>,
        /// Process exit code.
        code: Option<i32>,
    },
}

impl fmt::Display for TaskRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProjectMissing(path) => {
                write!(f, "managed project does not exist at {}", path.display())
            }
            Self::MissingTaskSource(path) => {
                write!(f, "task source README does not exist at {}", path.display())
            }
            Self::MissingTaskId(path) => {
                write!(
                    f,
                    "task source README does not define a task id at {}",
                    path.display()
                )
            }
            Self::InvalidTaskId(error) => write!(f, "{error}"),
            Self::MissingProjectConfig(path) => {
                write!(f, "project config does not exist at {}", path.display())
            }
            Self::UnknownRepository(path) => {
                write!(
                    f,
                    "project config does not define a GitHub repository at {}",
                    path.display()
                )
            }
            Self::GitHubTaskFailed {
                repo,
                task_id,
                code,
            } => {
                write!(
                    f,
                    "failed to load GitHub task {repo}#{task_id} with exit code {code:?}"
                )
            }
            Self::GitHubCommentFailed {
                repo,
                task_id,
                code,
            } => {
                write!(
                    f,
                    "failed to post GitHub blocker comment on {repo}#{task_id} with exit code {code:?}"
                )
            }
            Self::RuntimeRepositoryInvalid {
                source_repo,
                runtime_repo,
            } => {
                write!(
                    f,
                    "runtime repository {runtime_repo} must be separate from target repository {source_repo}"
                )
            }
            Self::RuntimeRepositoryFailed {
                runtime_repo,
                action,
                code,
            } => {
                write!(
                    f,
                    "runtime repository {runtime_repo} failed during {action} with exit code {code:?}"
                )
            }
            Self::PreflightFailed { check, next_action } => {
                write!(
                    f,
                    "preflight check failed: {check}; next action: {next_action}"
                )
            }
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} at {}: {source}", path.display()),
            Self::GitSpawn { cwd, source } => {
                write!(f, "failed to launch git in {}: {source}", cwd.display())
            }
            Self::GitFailed { cwd, args, code } => {
                let args = args.join(" ");
                write!(
                    f,
                    "git {args} failed in {} with exit code {code:?}",
                    cwd.display()
                )
            }
        }
    }
}

impl std::error::Error for TaskRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTaskId(error) => Some(error),
            Self::Io { source, .. } | Self::GitSpawn { source, .. } => Some(source),
            Self::ProjectMissing(_)
            | Self::MissingTaskSource(_)
            | Self::MissingTaskId(_)
            | Self::MissingProjectConfig(_)
            | Self::UnknownRepository(_)
            | Self::GitHubTaskFailed { .. }
            | Self::GitHubCommentFailed { .. }
            | Self::RuntimeRepositoryInvalid { .. }
            | Self::RuntimeRepositoryFailed { .. }
            | Self::PreflightFailed { .. }
            | Self::GitFailed { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        fs, io,
        path::{Path, PathBuf},
    };

    use super::{GIT_REMOTE_GET_URL_ARG, READ_BEST_METADATA_ACTION, REV_PARSE_ARG, VERIFY_ARG};
    use crate::{
        BestRecord, CyanosHome, Evaluation, ProjectId, QualityTier, SampleScore, ScoreBreakdown,
        SystemTaskGitRunner, TaskGitRunner, TaskId, TaskLayout, TaskResult, TaskRunError,
        TaskRunRequest, TaskRunner, VerifierFeedback,
        terms::{ORIGIN_REMOTE, REMOTE_ARG, WORKTREE_ARG},
        write_best_record,
    };

    #[derive(Debug)]
    struct RecordingTaskGitRunner {
        calls: RefCell<Vec<(PathBuf, Vec<String>)>>,
        available_refs: Vec<String>,
        has_remote: bool,
        fail: bool,
    }

    impl RecordingTaskGitRunner {
        fn new(has_remote: bool) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                available_refs: Vec::new(),
                has_remote,
                fail: false,
            }
        }

        fn with_available_ref(mut self, reference: &str) -> Self {
            self.available_refs.push(reference.to_owned());
            self
        }

        fn failing() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                available_refs: Vec::new(),
                has_remote: false,
                fail: true,
            }
        }

        fn calls(&self) -> Vec<(PathBuf, Vec<String>)> {
            self.calls.borrow().clone()
        }
    }

    impl TaskGitRunner for RecordingTaskGitRunner {
        fn run(&self, cwd: &Path, args: &[&str]) -> Result<(), TaskRunError> {
            self.calls.borrow_mut().push((
                cwd.to_path_buf(),
                args.iter().map(|arg| (*arg).to_owned()).collect(),
            ));

            if self.fail {
                Err(TaskRunError::GitFailed {
                    cwd: cwd.to_path_buf(),
                    args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                    code: Some(1),
                })
            } else {
                Ok(())
            }
        }

        fn succeeds(&self, cwd: &Path, args: &[&str]) -> Result<bool, TaskRunError> {
            self.calls.borrow_mut().push((
                cwd.to_path_buf(),
                args.iter().map(|arg| (*arg).to_owned()).collect(),
            ));
            if args.first().copied() == Some(REMOTE_ARG)
                && args.get(1).copied() == Some(GIT_REMOTE_GET_URL_ARG)
                && args.get(2).copied() == Some(ORIGIN_REMOTE)
            {
                return Ok(self.has_remote);
            }
            if args.first().copied() == Some(REV_PARSE_ARG)
                && args.get(1).copied() == Some(VERIFY_ARG)
            {
                return Ok(args.get(2).is_some_and(|reference| {
                    self.available_refs
                        .iter()
                        .any(|available| available == reference)
                }));
            }
            Ok(false)
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("cyanos-task-{name}-{}", std::process::id()))
    }

    fn request(root: &Path) -> Result<TaskRunRequest, crate::IdentifierError> {
        Ok(TaskRunRequest::new(
            CyanosHome::new(root.join("home/.cyanos")),
            ProjectId::new("demo")?,
            TaskId::new("123".to_owned())?,
        ))
    }

    fn write_task_source(root: &Path) -> io::Result<()> {
        let readme = root.join("home/.cyanos/projects/demo/origin/.cyanos/README.md");
        let parent = readme
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| io::Error::other("invalid README path"))?;
        fs::create_dir_all(parent)?;
        fs::write(
            readme,
            "Task ID: 123\n\nBuild a useful product from user intent.\n",
        )
    }

    fn args_eq(args: &[String], expected: &[&str]) -> bool {
        args.iter().map(String::as_str).eq(expected.iter().copied())
    }

    fn score(value: i64, tier: QualityTier) -> ScoreBreakdown {
        ScoreBreakdown::new(
            SampleScore::new(value),
            tier,
            SampleScore::new(value),
            SampleScore::new(value),
            SampleScore::new(value),
        )
    }

    fn write_completed_sample_evidence(
        task: &TaskLayout,
        sample_id: usize,
        run_index: usize,
    ) -> io::Result<()> {
        fs::create_dir_all(task.sample_run_worktree(sample_id, run_index))?;
        fs::write(
            task.sample_eval(sample_id, run_index),
            Evaluation::accepted()
                .with_evidence(
                    vec!["cargo_check: passed".to_owned()],
                    vec!["coverage: 9500".to_owned()],
                )
                .to_json(),
        )?;
        fs::write(task.sample_summary(sample_id, run_index), "sample summary")?;
        fs::write(
            task.sample_patch(sample_id, run_index),
            "diff --git a/src/lib.rs b/src/lib.rs",
        )?;
        let mut result = TaskResult::new(
            score(0, QualityTier::CompileFailed),
            score(10_000, QualityTier::Passed),
        );
        result.record_run(
            run_index,
            sample_id,
            score(9_500, QualityTier::Passed),
            VerifierFeedback::new(
                format!("runs/{run_index}/samples/{sample_id}/eval.json"),
                "sample passed repository verifier".to_owned(),
                "none".to_owned(),
            )
            .with_next_action("promote verified candidate".to_owned()),
        );
        fs::write(task.result(), result.to_json())
    }

    #[test]
    fn bootstraps_task_worktree_from_project_task_id() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("bootstrap");
        write_task_source(&root)?;
        let git = RecordingTaskGitRunner::new(true);
        let runner = TaskRunner::new(&git);

        let report = runner.bootstrap(&request(&root)?)?;

        assert_eq!(report.task().task_id().as_str(), "123");
        assert_eq!(report.task().evolution_branch_name(), "cyanos/123");
        assert_eq!(report.task().pr_branch_name(), "feature/123");
        assert!(
            report
                .source_readme()
                .ends_with(Path::new(".cyanos/README.md"))
        );
        assert!(report.task().readme().is_file());
        let readme = fs::read_to_string(report.task().readme())?;
        assert!(readme.contains("Project: demo"));
        assert!(readme.contains("Evolution Branch: cyanos/123"));
        assert!(readme.contains("PR Branch: feature/123"));
        assert!(readme.contains("Best:"));
        assert!(readme.contains("tasks/123/best.json"));
        assert!(readme.contains("Task ID: 123"));
        assert!(readme.contains("Build a useful product from user intent."));
        let calls = git.calls();
        assert!(
            calls
                .iter()
                .any(|call| args_eq(&call.1, &["remote", "get-url", "origin"]))
        );
        assert!(
            calls
                .iter()
                .any(|call| args_eq(&call.1, &["fetch", "--all", "--prune"]))
        );
        assert!(
            calls
                .iter()
                .any(|call| { args_eq(&call.1, &["remote", "set-head", "origin", "--auto"]) })
        );
        assert!(calls.iter().any(|call| {
            call.1.first().map(String::as_str) == Some("worktree")
                && call.1.get(1).map(String::as_str) == Some("add")
                && call.1.get(2).map(String::as_str) == Some("--detach")
                && call
                    .1
                    .get(3)
                    .is_some_and(|path| path.ends_with("runs/1/samples/1/worktree"))
                && call.1.last().map(String::as_str) == Some("origin/HEAD")
        }));
        assert!(
            !calls
                .iter()
                .any(|call| { call.1.iter().any(|arg| arg.ends_with("tasks/123/worktree")) })
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn bootstraps_task_worktree_from_explicit_task_brief() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = temp_root("explicit-brief");
        fs::create_dir_all(root.join("home/.cyanos/projects/demo/origin"))?;
        let git = RecordingTaskGitRunner::new(false);
        let runner = TaskRunner::new(&git);
        let run_request = request(&root)?;

        let report = runner.bootstrap_task(
            &run_request,
            PathBuf::from("owner/repo/issues/123"),
            "## Problem\n\nBuild from GitHub issue content.\n".to_owned(),
            2,
        )?;

        assert_eq!(report.task().task_id().as_str(), "123");
        assert!(report.task().readme().is_file());
        assert!(
            fs::read_to_string(report.task().readme())?
                .contains("Build from GitHub issue content.")
        );
        assert!(
            report
                .task()
                .sample_run_worktree(1, 1)
                .ends_with("worktree")
        );
        assert!(
            report
                .task()
                .sample_run_worktree(2, 1)
                .ends_with("worktree")
        );
        fs::create_dir_all(report.task().sample_run_worktree(1, 2))?;
        runner.bootstrap_sample_run(&run_request, 2, 2)?;
        let calls = git.calls();
        assert!(calls.iter().any(|call| {
            call.1.first().map(String::as_str) == Some("worktree")
                && call.1.get(1).map(String::as_str) == Some("remove")
                && call.1.get(2).map(String::as_str) == Some("--force")
                && call
                    .1
                    .get(3)
                    .is_some_and(|path| path.ends_with("runs/2/samples/1/worktree"))
        }));
        assert!(calls.iter().any(|call| {
            call.1.first().map(String::as_str) == Some("worktree")
                && call.1.get(1).map(String::as_str) == Some("add")
                && call.1.get(2).map(String::as_str) == Some("--detach")
                && call
                    .1
                    .get(3)
                    .is_some_and(|path| path.ends_with("runs/2/samples/1/worktree"))
                && call.1.last().map(String::as_str) == Some("HEAD")
        }));
        assert!(calls.iter().any(|call| {
            call.1.first().map(String::as_str) == Some("worktree")
                && call.1.get(1).map(String::as_str) == Some("add")
                && call.1.get(2).map(String::as_str) == Some("--detach")
                && call
                    .1
                    .get(3)
                    .is_some_and(|path| path.ends_with("runs/2/samples/2/worktree"))
                && call.1.last().map(String::as_str) == Some("HEAD")
        }));

        let missing_root = temp_root("explicit-brief-missing-project");
        let missing = runner.bootstrap_task(
            &request(&missing_root)?,
            PathBuf::from("owner/repo/issues/123"),
            "body".to_owned(),
            1,
        );
        assert!(matches!(missing, Err(TaskRunError::ProjectMissing(_))));
        let missing_sample_run = runner.bootstrap_sample_run(&request(&missing_root)?, 2, 1);
        assert!(matches!(
            missing_sample_run,
            Err(TaskRunError::ProjectMissing(_))
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn reuses_sample_worktree_only_with_valid_eval_and_result_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("sample-evidence");
        fs::create_dir_all(root.join("home/.cyanos/projects/demo/origin"))?;
        let run_request = request(&root)?;
        let initial_git = RecordingTaskGitRunner::new(false);
        let initial_runner = TaskRunner::new(&initial_git);
        let report = initial_runner.bootstrap_task(
            &run_request,
            PathBuf::from("owner/repo/issues/123"),
            "## Problem\n\nBuild from GitHub issue content.\n".to_owned(),
            1,
        )?;
        let task = report.task();

        fs::create_dir_all(task.sample_run_worktree(1, 2))?;
        fs::write(task.sample_eval(1, 2), "{")?;
        fs::write(task.sample_summary(1, 2), "sample summary")?;
        fs::write(task.sample_patch(1, 2), "diff --git")?;
        let invalid_git = RecordingTaskGitRunner::new(false);
        TaskRunner::new(&invalid_git).bootstrap_sample_run(&run_request, 2, 1)?;
        assert!(invalid_git.calls().iter().any(|call| {
            call.1.first().map(String::as_str) == Some("worktree")
                && call.1.get(1).map(String::as_str) == Some("remove")
                && call.1.get(2).map(String::as_str) == Some("--force")
        }));

        write_completed_sample_evidence(task, 1, 2)?;
        fs::remove_file(task.result())?;
        let missing_result_git = RecordingTaskGitRunner::new(false);
        TaskRunner::new(&missing_result_git).bootstrap_sample_run(&run_request, 2, 1)?;
        assert!(missing_result_git.calls().iter().any(|call| {
            call.1.first().map(String::as_str) == Some("worktree")
                && call.1.get(1).map(String::as_str) == Some("remove")
        }));

        write_completed_sample_evidence(task, 1, 2)?;
        let valid_git = RecordingTaskGitRunner::new(false);
        TaskRunner::new(&valid_git).bootstrap_sample_run(&run_request, 2, 1)?;
        assert!(!valid_git.calls().iter().any(|call| {
            call.1.first().map(String::as_str) == Some("worktree")
                && matches!(call.1.get(1).map(String::as_str), Some("remove" | "add"))
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn resumes_sample_run_from_best_json_commit_when_branch_is_missing()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("best-resume");
        fs::create_dir_all(root.join("home/.cyanos/projects/demo/origin"))?;
        let run_request = request(&root)?;
        let report = TaskRunner::new(RecordingTaskGitRunner::new(false)).bootstrap_task(
            &run_request,
            PathBuf::from("owner/repo/issues/123"),
            "## Problem\n\nBuild from GitHub issue content.\n".to_owned(),
            1,
        )?;
        let task = report.task();
        write_best_record(
            &task.best(),
            &BestRecord::new(
                "123",
                1,
                1,
                score(9_500, QualityTier::Passed),
                "runs/1/samples/1/eval.json",
                "runs/1/samples/1/patch.diff",
                "targetabc",
                "cyanosabc",
                "prabc",
            ),
        )?;

        let git = RecordingTaskGitRunner::new(false).with_available_ref("cyanosabc^{commit}");
        TaskRunner::new(&git).bootstrap_sample_run(&run_request, 2, 1)?;

        assert!(git.calls().iter().any(|call| {
            call.1.first().map(String::as_str) == Some(WORKTREE_ARG)
                && call.1.get(1).map(String::as_str) == Some("add")
                && call.1.get(2).map(String::as_str) == Some("--detach")
                && call.1.last().map(String::as_str) == Some("cyanosabc")
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn uses_local_evolution_branch_before_best_json_or_remote()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("best-local-branch");
        fs::create_dir_all(root.join("home/.cyanos/projects/demo/origin"))?;
        let run_request = request(&root)?;
        let report = TaskRunner::new(RecordingTaskGitRunner::new(false)).bootstrap_task(
            &run_request,
            PathBuf::from("owner/repo/issues/123"),
            "## Problem\n\nBuild from GitHub issue content.\n".to_owned(),
            1,
        )?;
        let task = report.task();
        let branch = task.evolution_branch_name();
        write_best_record(
            &task.best(),
            &BestRecord::new(
                "123",
                1,
                1,
                score(9_500, QualityTier::Passed),
                "runs/1/samples/1/eval.json",
                "runs/1/samples/1/patch.diff",
                "targetabc",
                "cyanosabc",
                "prabc",
            ),
        )?;

        let git = RecordingTaskGitRunner::new(true).with_available_ref(&branch);
        TaskRunner::new(&git).bootstrap_sample_run(&run_request, 2, 1)?;

        assert!(git.calls().iter().any(|call| {
            call.1.first().map(String::as_str) == Some(WORKTREE_ARG)
                && call.1.get(1).map(String::as_str) == Some("add")
                && call.1.last().map(String::as_str) == Some(branch.as_str())
        }));
        assert!(!git.calls().iter().any(|call| {
            call.1.first().map(String::as_str) == Some(REMOTE_ARG)
                && call.1.get(1).map(String::as_str) == Some(GIT_REMOTE_GET_URL_ARG)
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn resumes_sample_run_from_target_commit_when_cyanos_commit_is_empty()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("best-target-resume");
        fs::create_dir_all(root.join("home/.cyanos/projects/demo/origin"))?;
        let run_request = request(&root)?;
        let report = TaskRunner::new(RecordingTaskGitRunner::new(false)).bootstrap_task(
            &run_request,
            PathBuf::from("owner/repo/issues/123"),
            "## Problem\n\nBuild from GitHub issue content.\n".to_owned(),
            1,
        )?;
        let task = report.task();
        write_best_record(
            &task.best(),
            &BestRecord::new(
                "123",
                1,
                1,
                score(9_500, QualityTier::Passed),
                "runs/1/samples/1/eval.json",
                "runs/1/samples/1/patch.diff",
                "targetabc",
                "",
                "prabc",
            ),
        )?;

        let git = RecordingTaskGitRunner::new(false).with_available_ref("targetabc^{commit}");
        TaskRunner::new(&git).bootstrap_sample_run(&run_request, 2, 1)?;

        assert!(git.calls().iter().any(|call| {
            call.1.first().map(String::as_str) == Some(WORKTREE_ARG)
                && call.1.get(1).map(String::as_str) == Some("add")
                && call.1.last().map(String::as_str) == Some("targetabc")
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn reports_malformed_best_json_before_falling_back_to_base()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("best-malformed");
        fs::create_dir_all(root.join("home/.cyanos/projects/demo/origin"))?;
        let run_request = request(&root)?;
        let report = TaskRunner::new(RecordingTaskGitRunner::new(false)).bootstrap_task(
            &run_request,
            PathBuf::from("owner/repo/issues/123"),
            "## Problem\n\nBuild from GitHub issue content.\n".to_owned(),
            1,
        )?;
        fs::write(report.task().best(), "{")?;

        let result = TaskRunner::new(RecordingTaskGitRunner::new(false)).bootstrap_sample_run(
            &run_request,
            2,
            1,
        );

        assert!(matches!(
            result,
            Err(TaskRunError::Io { action, .. }) if action == READ_BEST_METADATA_ACTION
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn reports_missing_best_json_commit_before_falling_back_to_base()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("best-missing-commit");
        fs::create_dir_all(root.join("home/.cyanos/projects/demo/origin"))?;
        let run_request = request(&root)?;
        let report = TaskRunner::new(RecordingTaskGitRunner::new(false)).bootstrap_task(
            &run_request,
            PathBuf::from("owner/repo/issues/123"),
            "## Problem\n\nBuild from GitHub issue content.\n".to_owned(),
            1,
        )?;
        let task = report.task();
        write_best_record(
            &task.best(),
            &BestRecord::new(
                "123",
                1,
                1,
                score(9_500, QualityTier::Passed),
                "runs/1/samples/1/eval.json",
                "runs/1/samples/1/patch.diff",
                "missingtarget",
                "missingcyanos",
                "missingpr",
            ),
        )?;

        let result = TaskRunner::new(RecordingTaskGitRunner::new(false)).bootstrap_sample_run(
            &run_request,
            2,
            1,
        );

        assert!(matches!(result, Err(TaskRunError::PreflightFailed { .. })));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn incomplete_sample_cleanup_stops_when_worktree_remove_fails()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("sample-cleanup-failure");
        fs::create_dir_all(root.join("home/.cyanos/projects/demo/origin"))?;
        let run_request = request(&root)?;
        let setup_git = RecordingTaskGitRunner::new(false);
        let report = TaskRunner::new(&setup_git).bootstrap_task(
            &run_request,
            PathBuf::from("owner/repo/issues/123"),
            "## Problem\n\nBuild from GitHub issue content.\n".to_owned(),
            1,
        )?;
        let task = report.task();
        fs::create_dir_all(task.sample_run_worktree(1, 2))?;
        fs::write(task.sample_summary(1, 2), "partial sample")?;
        let failing_git = RecordingTaskGitRunner::failing();

        let result = TaskRunner::new(&failing_git).bootstrap_sample_run(&run_request, 2, 1);

        assert!(matches!(result, Err(TaskRunError::GitFailed { .. })));
        assert!(task.sample_run_root(1, 2).exists());
        assert!(failing_git.calls().iter().any(|call| {
            call.1.first().map(String::as_str) == Some("worktree")
                && call.1.get(1).map(String::as_str) == Some("remove")
                && call.1.get(2).map(String::as_str) == Some("--force")
        }));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn skips_fetch_when_origin_has_no_remote() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("no-remote");
        write_task_source(&root)?;
        let git = RecordingTaskGitRunner::new(false);
        let runner = TaskRunner::new(&git);

        let report = runner.bootstrap(&request(&root)?)?;

        assert_eq!(report.task().task_id().as_str(), "123");
        assert!(
            !git.calls()
                .iter()
                .any(|call| args_eq(&call.1, &["fetch", "--all", "--prune"]))
        );
        assert!(
            !git.calls()
                .iter()
                .any(|call| args_eq(&call.1, &["remote", "set-head", "origin", "--auto"]))
        );
        assert!(git.calls().iter().any(|call| {
            call.1.first().map(String::as_str) == Some("worktree")
                && call.1.last().map(String::as_str) == Some("HEAD")
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_missing_project_and_task_source() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("missing");
        let git = RecordingTaskGitRunner::new(false);
        let runner = TaskRunner::new(&git);

        let missing_project = runner.bootstrap(&request(&root)?);
        assert!(matches!(
            missing_project,
            Err(TaskRunError::ProjectMissing(_))
        ));

        fs::create_dir_all(root.join("home/.cyanos/projects/demo/origin"))?;
        let missing_source = runner.bootstrap(&request(&root)?);
        assert!(matches!(
            missing_source,
            Err(TaskRunError::MissingTaskSource(_))
        ));

        let readme = root.join("home/.cyanos/projects/demo/origin/.cyanos/README.md");
        fs::create_dir_all(
            readme
                .parent()
                .ok_or_else(|| io::Error::other("invalid README path"))?,
        )?;
        fs::write(&readme, "Build a useful product from user intent.\n")?;
        let missing_task_id = runner.bootstrap(&request(&root)?);
        assert!(matches!(
            missing_task_id,
            Err(TaskRunError::MissingTaskId(_))
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn reports_git_and_io_errors() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("errors");
        write_task_source(&root)?;
        let runner = TaskRunner::new(RecordingTaskGitRunner::failing());

        let result = runner.bootstrap(&request(&root)?);

        match result {
            Err(error @ TaskRunError::GitFailed { .. }) => {
                assert!(error.to_string().contains("git worktree add"));
                assert!(std::error::Error::source(&error).is_none());
            }
            other => {
                return Err(io::Error::other(format!("unexpected result: {other:?}")).into());
            }
        }
        assert!(
            TaskRunError::MissingTaskSource(PathBuf::from("/tmp/missing"))
                .to_string()
                .contains("task source README")
        );
        assert!(
            TaskRunError::MissingTaskId(PathBuf::from("/tmp/missing"))
                .to_string()
                .contains("does not define a task id")
        );
        assert!(
            TaskRunError::ProjectMissing(PathBuf::from("/tmp/project"))
                .to_string()
                .contains("managed project")
        );
        assert!(
            TaskRunError::MissingProjectConfig(PathBuf::from("/tmp/config.txt"))
                .to_string()
                .contains("project config does not exist")
        );
        assert!(
            TaskRunError::UnknownRepository(PathBuf::from("/tmp/config.txt"))
                .to_string()
                .contains("does not define a GitHub repository")
        );
        assert!(
            TaskRunError::GitHubTaskFailed {
                repo: "owner/repo".to_owned(),
                task_id: "123".to_owned(),
                code: Some(7),
            }
            .to_string()
            .contains("failed to load GitHub task owner/repo#123")
        );
        assert!(
            TaskRunError::GitHubCommentFailed {
                repo: "owner/repo".to_owned(),
                task_id: "123".to_owned(),
                code: Some(8),
            }
            .to_string()
            .contains("failed to post GitHub blocker comment")
        );
        assert!(
            TaskRunError::RuntimeRepositoryInvalid {
                source_repo: "owner/repo".to_owned(),
                runtime_repo: "owner/repo".to_owned(),
            }
            .to_string()
            .contains("must be separate")
        );
        assert!(
            TaskRunError::RuntimeRepositoryFailed {
                runtime_repo: "owner/cyanos-repo".to_owned(),
                action: "push",
                code: Some(1),
            }
            .to_string()
            .contains("failed during push")
        );
        let io_error = TaskRunError::Io {
            action: "read",
            path: PathBuf::from("/tmp/file"),
            source: io::Error::other("denied"),
        };
        let git_spawn = TaskRunError::GitSpawn {
            cwd: PathBuf::from("/tmp/repo"),
            source: io::Error::other("missing"),
        };
        assert!(io_error.to_string().contains("failed to read"));
        assert!(git_spawn.to_string().contains("failed to launch git"));
        assert!(std::error::Error::source(&io_error).is_some());
        assert!(std::error::Error::source(&git_spawn).is_some());

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn reports_non_missing_task_source_read_errors() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("source-read-error");
        let readme = root.join("home/.cyanos/projects/demo/origin/.cyanos/README.md");
        fs::create_dir_all(&readme)?;
        let runner = TaskRunner::new(RecordingTaskGitRunner::new(false));

        let result = runner.bootstrap(&request(&root)?);

        assert!(matches!(result, Err(TaskRunError::Io { .. })));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn reports_invalid_task_id_sources() -> Result<(), Box<dyn std::error::Error>> {
        let result = TaskId::new(String::new()).map_err(TaskRunError::InvalidTaskId);

        let Err(error) = result else {
            return Err(io::Error::other("expected invalid task id").into());
        };
        assert!(error.to_string().contains("invalid task id"));
        assert!(std::error::Error::source(&error).is_some());
        Ok(())
    }

    #[test]
    fn system_git_runner_reports_command_success_and_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("system-git");
        fs::create_dir_all(&root)?;
        let git = SystemTaskGitRunner;

        assert!(git.succeeds(&root, &["--version"])?);
        let result = git.run(&root, &["definitely-not-a-cyanos-command"]);
        assert!(matches!(result, Err(TaskRunError::GitFailed { .. })));
        let spawn = git.succeeds(&root.join("missing"), &["--version"]);
        assert!(matches!(spawn, Err(TaskRunError::GitSpawn { .. })));

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
