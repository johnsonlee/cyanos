//! Runtime workspace layout for Cyanos projects and tasks.

use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

use crate::terms::{
    BEST_FILE, CONFIG_FILE, CYANOS_LOCK_FILE, LEDGER_FILE, ORIGIN_DIR, PATCH_FILE, PROJECTS_DIR,
    PROMPT_SNAPSHOT_PREFIX, README_FILE, RESULT_FILE, RUNS_DIR, SAMPLE_EVAL_FILE, SAMPLES_DIR,
    SUMMARY_FILE, TASKS_DIR, WORKTREE_DIR,
};

const GIT_SUFFIX: &str = ".git";

/// Identifier validation error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentifierError {
    kind: &'static str,
    value: String,
}

impl IdentifierError {
    fn new(kind: &'static str, value: String) -> Self {
        Self { kind, value }
    }
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = self.kind;
        if kind == "project locator" {
            write!(
                f,
                "invalid {kind}: use '<owner>/<repo>' or a full http(s) repository URL"
            )
        } else {
            write!(f, "invalid {kind}: use ASCII letters, digits, '-' or '_'")
        }
    }
}

impl std::error::Error for IdentifierError {}

fn validate_identifier(kind: &'static str, value: String) -> Result<String, IdentifierError> {
    let valid = !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');

    if valid {
        Ok(value)
    } else {
        Err(IdentifierError::new(kind, value))
    }
}

/// Cyanos project id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectId {
    value: String,
    path_segment: String,
}

impl ProjectId {
    /// Creates a project id.
    ///
    /// # Errors
    ///
    /// Returns an error when the locator is empty, unsupported, or unsafe.
    pub fn new(value: impl AsRef<str>) -> Result<Self, IdentifierError> {
        normalize_project_locator(value.as_ref()).map(|(value, path_segment)| Self {
            value,
            path_segment,
        })
    }

    /// Returns the project id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Returns the path-safe runtime directory segment for this project.
    #[must_use]
    pub fn path_segment(&self) -> &str {
        &self.path_segment
    }
}

fn normalize_project_locator(value: &str) -> Result<(String, String), IdentifierError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(IdentifierError::new("project locator", value.to_owned()));
    }

    if let Some((scheme, rest)) = value.split_once("://") {
        return normalize_project_url(scheme, rest)
            .ok_or_else(|| IdentifierError::new("project locator", value.to_owned()));
    }

    if let Some((owner, repo)) = value.split_once('/') {
        return if value.matches('/').count() == 1
            && valid_repo_component(owner)
            && valid_repo_component(repo)
        {
            Ok((
                format!("{owner}/{repo}"),
                format!(
                    "{}__{}",
                    path_safe_component(owner),
                    path_safe_component(repo)
                ),
            ))
        } else {
            Err(IdentifierError::new("project locator", value.to_owned()))
        };
    }

    validate_identifier("project id", value.to_owned()).map(|value| (value.clone(), value))
}

fn normalize_project_url(scheme: &str, rest: &str) -> Option<(String, String)> {
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return None;
    }

    let normalized = rest.trim_end_matches('/').trim_end_matches(GIT_SUFFIX);
    let mut parts = normalized.split('/');
    let host = parts.next()?;
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next().is_some()
        || !valid_host(host)
        || !valid_repo_component(owner)
        || !valid_repo_component(repo)
    {
        return None;
    }

    let host = host.to_ascii_lowercase();
    Some((
        format!("{scheme}://{host}/{owner}/{repo}"),
        format!(
            "{}__{}__{}__{}",
            scheme,
            path_safe_component(&host),
            path_safe_component(owner),
            path_safe_component(repo)
        ),
    ))
}

fn valid_host(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

fn valid_repo_component(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn path_safe_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
                char::from(byte)
            } else {
                '_'
            }
        })
        .collect()
}

/// Cyanos task id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskId {
    value: String,
}

impl TaskId {
    /// Creates a task id.
    ///
    /// # Errors
    ///
    /// Returns an error when the id is empty or not path-safe.
    pub fn new(value: String) -> Result<Self, IdentifierError> {
        validate_identifier("task id", value).map(|value| Self { value })
    }

    /// Returns the task id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// Root runtime directory, usually `~/.cyanos`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CyanosHome {
    root: PathBuf,
}

impl CyanosHome {
    /// Creates a runtime home.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Returns the runtime root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the projects directory.
    #[must_use]
    pub fn projects(&self) -> PathBuf {
        self.root.join(PROJECTS_DIR)
    }

    /// Returns a project layout.
    #[must_use]
    pub fn project(&self, project_id: ProjectId) -> ProjectLayout {
        ProjectLayout::new(self.clone(), project_id)
    }
}

/// Runtime layout for one project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectLayout {
    home: CyanosHome,
    project_id: ProjectId,
}

impl ProjectLayout {
    /// Creates a project layout.
    #[must_use]
    pub fn new(home: CyanosHome, project_id: ProjectId) -> Self {
        Self { home, project_id }
    }

    /// Returns the project id.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns `~/.cyanos/projects/<project-id>`.
    #[must_use]
    pub fn root(&self) -> PathBuf {
        self.home.projects().join(self.project_id.path_segment())
    }

    /// Returns the managed origin repository path.
    #[must_use]
    pub fn origin(&self) -> PathBuf {
        self.root().join(ORIGIN_DIR)
    }

    /// Returns the managed project configuration file path.
    #[must_use]
    pub fn config(&self) -> PathBuf {
        self.root().join(CONFIG_FILE)
    }

    /// Returns the task directory root.
    #[must_use]
    pub fn tasks(&self) -> PathBuf {
        self.root().join(TASKS_DIR)
    }

    /// Returns the project lock path.
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.root().join(CYANOS_LOCK_FILE)
    }

    /// Returns a task layout.
    #[must_use]
    pub fn task(&self, task_id: TaskId) -> TaskLayout {
        TaskLayout::new(self.clone(), task_id)
    }
}

/// Runtime layout for one task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLayout {
    project: ProjectLayout,
    task_id: TaskId,
}

impl TaskLayout {
    /// Creates a task layout.
    #[must_use]
    pub fn new(project: ProjectLayout, task_id: TaskId) -> Self {
        Self { project, task_id }
    }

    /// Returns the task id.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the owning project layout.
    #[must_use]
    pub const fn project(&self) -> &ProjectLayout {
        &self.project
    }

    /// Returns `~/.cyanos/projects/<project-id>/tasks/<task-id>`.
    #[must_use]
    pub fn root(&self) -> PathBuf {
        self.project.tasks().join(self.task_id.as_str())
    }

    /// Returns the temporary materialized task checkout used for promotion and PR delivery.
    #[must_use]
    pub fn worktree(&self) -> PathBuf {
        self.root().join(WORKTREE_DIR)
    }

    /// Returns the task details file generated by `run`.
    #[must_use]
    pub fn readme(&self) -> PathBuf {
        self.root().join(README_FILE)
    }

    /// Returns the human-readable sample summary.
    #[must_use]
    pub fn summary(&self) -> PathBuf {
        self.sample_summary(1, 1)
    }

    /// Returns the repository evolution branch name for this task.
    #[must_use]
    pub fn evolution_branch_name(&self) -> String {
        format!("cyanos/{}", self.task_id.as_str())
    }

    /// Returns the PR code branch name for this task.
    #[must_use]
    pub fn pr_branch_name(&self) -> String {
        format!("feature/{}", self.task_id.as_str())
    }

    /// Returns the current task worktree branch name.
    #[must_use]
    pub fn branch_name(&self) -> String {
        self.evolution_branch_name()
    }

    /// Returns the runtime prompt snapshot path for one outer run.
    #[must_use]
    pub fn prompt_snapshot(&self, run_index: usize) -> PathBuf {
        self.root()
            .join(format!("{PROMPT_SNAPSHOT_PREFIX}.{run_index}.md"))
    }

    /// Returns the task-level evolution ledger.
    #[must_use]
    pub fn ledger(&self) -> PathBuf {
        self.root().join(LEDGER_FILE)
    }

    /// Returns the task-level result state file.
    #[must_use]
    pub fn result(&self) -> PathBuf {
        self.root().join(RESULT_FILE)
    }

    /// Returns the global best metadata file.
    #[must_use]
    pub fn best(&self) -> PathBuf {
        self.root().join(BEST_FILE)
    }

    /// Returns the directory for one outer run.
    #[must_use]
    pub fn run_root(&self, run_index: usize) -> PathBuf {
        self.root().join(RUNS_DIR).join(run_index.to_string())
    }

    /// Returns the directory for one sample inside one outer run.
    #[must_use]
    pub fn sample_run_root(&self, sample_id: usize, run_index: usize) -> PathBuf {
        self.run_root(run_index)
            .join(SAMPLES_DIR)
            .join(sample_id.to_string())
    }

    /// Returns a sample run worktree path.
    #[must_use]
    pub fn sample_run_worktree(&self, sample_id: usize, run_index: usize) -> PathBuf {
        self.sample_run_root(sample_id, run_index)
            .join(WORKTREE_DIR)
    }

    /// Returns a human-readable sample summary path.
    #[must_use]
    pub fn sample_summary(&self, sample_id: usize, run_index: usize) -> PathBuf {
        self.sample_run_root(sample_id, run_index)
            .join(SUMMARY_FILE)
    }

    /// Returns the sample evaluation result path.
    #[must_use]
    pub fn sample_eval(&self, sample_id: usize, run_index: usize) -> PathBuf {
        self.sample_run_root(sample_id, run_index)
            .join(SAMPLE_EVAL_FILE)
    }

    /// Returns the sample source diff path.
    #[must_use]
    pub fn sample_patch(&self, sample_id: usize, run_index: usize) -> PathBuf {
        self.sample_run_root(sample_id, run_index).join(PATCH_FILE)
    }
}

/// Global best promotion strategy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PromotionStrategy {
    /// Promote by materializing the selected sample patch into a temporary
    /// task checkout and writing durable `best.json` metadata.
    #[default]
    WorktreeSwap,
}

/// Guard for one active Cyanos process in a project.
#[derive(Debug)]
pub struct ProjectLock {
    path: PathBuf,
}

impl ProjectLock {
    /// Atomically acquires the project lock.
    ///
    /// # Errors
    ///
    /// Returns an error when the project is missing, already locked, or the
    /// lock file cannot be created.
    pub fn acquire(project: &ProjectLayout) -> Result<Self, ProjectLockError> {
        let root = project.root();
        if !root.is_dir() {
            return Err(ProjectLockError::ProjectMissing(root));
        }

        let path = project.lock_file();
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                if stale_lock_can_be_recovered(&path) {
                    fs::remove_file(&path).map_err(|source| ProjectLockError::Io {
                        action: "remove stale project lock",
                        path: path.clone(),
                        source,
                    })?;
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map_err(|source| ProjectLockError::Io {
                            action: "create project lock",
                            path: path.clone(),
                            source,
                        })?
                } else {
                    return Err(ProjectLockError::AlreadyLocked(path));
                }
            }
            Err(source) => {
                return Err(ProjectLockError::Io {
                    action: "create project lock",
                    path,
                    source,
                });
            }
        };

        writeln!(file, "pid={}", std::process::id()).map_err(|source| ProjectLockError::Io {
            action: "write project lock",
            path: path.clone(),
            source,
        })?;

        Ok(Self { path })
    }

    /// Returns the lock path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn stale_lock_can_be_recovered(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let Some(pid) = content
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|value| value.trim().parse::<u32>().ok())
    else {
        return false;
    };
    !process_is_alive(pid)
}

fn process_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Error returned when a project lock cannot be acquired.
#[derive(Debug)]
pub enum ProjectLockError {
    /// The managed project does not exist.
    ProjectMissing(PathBuf),
    /// Another Cyanos process already holds the lock.
    AlreadyLocked(PathBuf),
    /// Filesystem operation failed.
    Io {
        /// Action being performed.
        action: &'static str,
        /// Path being accessed.
        path: PathBuf,
        /// Original I/O error.
        source: io::Error,
    },
}

impl fmt::Display for ProjectLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProjectMissing(path) => {
                write!(f, "managed project does not exist at {}", path.display())
            }
            Self::AlreadyLocked(path) => {
                write!(f, "project is already locked at {}", path.display())
            }
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} at {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ProjectLockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::ProjectMissing(_) | Self::AlreadyLocked(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, path::PathBuf};

    use crate::{CyanosHome, ProjectId, ProjectLock, ProjectLockError, TaskId};

    #[test]
    fn builds_project_and_task_paths() -> Result<(), crate::IdentifierError> {
        let home = CyanosHome::new(PathBuf::from("/home/user/.cyanos"));
        let project = home.project(ProjectId::new("demo")?);
        let task = project.task(TaskId::new("task_1".to_owned())?);

        assert_eq!(home.root(), std::path::Path::new("/home/user/.cyanos"));
        assert_eq!(
            home.projects(),
            PathBuf::from("/home/user/.cyanos/projects")
        );
        assert_eq!(project.project_id().as_str(), "demo");
        assert_eq!(
            project.root(),
            PathBuf::from("/home/user/.cyanos/projects/demo")
        );
        assert_eq!(
            project.origin(),
            PathBuf::from("/home/user/.cyanos/projects/demo/origin")
        );
        assert_eq!(
            project.config(),
            PathBuf::from("/home/user/.cyanos/projects/demo/config.txt")
        );
        assert_eq!(
            project.tasks(),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks")
        );
        assert_eq!(
            project.lock_file(),
            PathBuf::from("/home/user/.cyanos/projects/demo/cyanos.lock")
        );
        assert_eq!(task.task_id().as_str(), "task_1");
        assert_eq!(
            task.root(),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks/task_1")
        );
        assert_eq!(
            task.worktree(),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks/task_1/worktree")
        );
        assert_eq!(
            task.readme(),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks/task_1/README.md")
        );
        assert_eq!(
            task.summary(),
            PathBuf::from(
                "/home/user/.cyanos/projects/demo/tasks/task_1/runs/1/samples/1/summary.md"
            )
        );
        assert_eq!(task.evolution_branch_name(), "cyanos/task_1");
        assert_eq!(task.pr_branch_name(), "feature/task_1");
        assert_eq!(task.branch_name(), task.evolution_branch_name());
        assert_eq!(
            task.prompt_snapshot(2),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks/task_1/prompt.2.md")
        );
        assert_eq!(
            task.ledger(),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks/task_1/ledger.jsonl")
        );
        assert_eq!(
            task.result(),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks/task_1/result.json")
        );
        assert_eq!(
            task.best(),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks/task_1/best.json")
        );
        assert_eq!(
            task.run_root(4),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks/task_1/runs/4")
        );
        assert_eq!(
            task.sample_run_root(3, 4),
            PathBuf::from("/home/user/.cyanos/projects/demo/tasks/task_1/runs/4/samples/3")
        );
        assert_eq!(
            task.sample_run_worktree(3, 4),
            PathBuf::from(
                "/home/user/.cyanos/projects/demo/tasks/task_1/runs/4/samples/3/worktree"
            )
        );
        assert_eq!(
            task.sample_summary(3, 4),
            PathBuf::from(
                "/home/user/.cyanos/projects/demo/tasks/task_1/runs/4/samples/3/summary.md"
            )
        );
        assert_eq!(
            task.sample_eval(3, 4),
            PathBuf::from(
                "/home/user/.cyanos/projects/demo/tasks/task_1/runs/4/samples/3/eval.json"
            )
        );
        assert_eq!(
            task.sample_patch(3, 4),
            PathBuf::from(
                "/home/user/.cyanos/projects/demo/tasks/task_1/runs/4/samples/3/patch.diff"
            )
        );

        Ok(())
    }

    #[test]
    fn builds_project_paths_from_repository_locators() -> Result<(), crate::IdentifierError> {
        let home = CyanosHome::new(PathBuf::from("/home/user/.cyanos"));
        let github = home.project(ProjectId::new("owner/repo")?);
        let enterprise = home.project(ProjectId::new("https://github.example.com/owner/repo.git")?);

        assert_eq!(github.project_id().as_str(), "owner/repo");
        assert_eq!(
            github.root(),
            PathBuf::from("/home/user/.cyanos/projects/owner__repo")
        );
        assert_eq!(
            enterprise.project_id().as_str(),
            "https://github.example.com/owner/repo"
        );
        assert_eq!(
            enterprise.root(),
            PathBuf::from("/home/user/.cyanos/projects/https__github_example_com__owner__repo")
        );

        Ok(())
    }

    #[test]
    fn rejects_path_unsafe_identifiers() -> Result<(), Box<dyn std::error::Error>> {
        let project = ProjectId::new("../repo");
        let empty_project = ProjectId::new("");
        let unsupported_url = ProjectId::new("ssh://github.com/owner/repo");
        let invalid_host = ProjectId::new("https://bad_host!/owner/repo");
        let task = TaskId::new(String::new());

        let Err(project_error) = project else {
            return Err(io::Error::other("expected invalid project id").into());
        };
        if empty_project.is_ok() {
            return Err(io::Error::other("expected empty project locator to fail").into());
        }
        if unsupported_url.is_ok() {
            return Err(io::Error::other("expected unsupported project URL to fail").into());
        }
        if invalid_host.is_ok() {
            return Err(io::Error::other("expected invalid project host to fail").into());
        }
        let Err(_task_error) = task else {
            return Err(io::Error::other("expected invalid task id").into());
        };

        assert_eq!(
            project_error.to_string(),
            "invalid project locator: use '<owner>/<repo>' or a full http(s) repository URL"
        );
        Ok(())
    }

    #[test]
    fn defaults_to_worktree_swap_promotion() {
        assert_eq!(
            crate::PromotionStrategy::default(),
            crate::PromotionStrategy::WorktreeSwap
        );
    }

    #[test]
    fn acquires_and_releases_project_lock() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("cyanos-lock-{}", std::process::id()));
        let project = CyanosHome::new(root.join("home")).project(ProjectId::new("demo")?);
        std::fs::create_dir_all(project.root())?;

        {
            let lock = ProjectLock::acquire(&project)?;
            assert_eq!(lock.path(), project.lock_file());
            let second = ProjectLock::acquire(&project);
            assert!(matches!(second, Err(ProjectLockError::AlreadyLocked(_))));
        };

        assert!(!project.lock_file().exists());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn recovers_stale_project_lock() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("cyanos-stale-lock-{}", std::process::id()));
        let project = CyanosHome::new(root.join("home")).project(ProjectId::new("demo")?);
        std::fs::create_dir_all(project.root())?;
        std::fs::write(project.lock_file(), "pid=999999\n")?;

        let lock = ProjectLock::acquire(&project)?;

        assert_eq!(lock.path(), project.lock_file());
        drop(lock);
        assert!(!project.lock_file().exists());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn stale_lock_detection_rejects_unreadable_or_live_lock_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("cyanos-lock-detect-{}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        let missing = root.join("missing.lock");
        let invalid = root.join("invalid.lock");
        let live = root.join("live.lock");
        std::fs::write(&invalid, "pid=not-a-number\n")?;
        std::fs::write(&live, format!("pid={}\n", std::process::id()))?;

        assert!(!super::stale_lock_can_be_recovered(&missing));
        assert!(!super::stale_lock_can_be_recovered(&invalid));
        assert!(!super::stale_lock_can_be_recovered(&live));

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn reports_project_lock_errors() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("cyanos-lock-missing-{}", std::process::id()));
        let project = CyanosHome::new(root.join("home")).project(ProjectId::new("demo")?);
        let missing = ProjectLock::acquire(&project);

        assert!(matches!(missing, Err(ProjectLockError::ProjectMissing(_))));
        let missing_error = ProjectLockError::ProjectMissing(PathBuf::from("/tmp/project"));
        assert!(missing_error.to_string().contains("managed project"));
        assert!(std::error::Error::source(&missing_error).is_none());
        let locked_error = ProjectLockError::AlreadyLocked(PathBuf::from("/tmp/cyanos.lock"));
        assert!(std::error::Error::source(&locked_error).is_none());
        assert!(locked_error.to_string().contains("already locked"));
        let io_error = ProjectLockError::Io {
            action: "create",
            path: PathBuf::from("/tmp/cyanos.lock"),
            source: std::io::Error::other("denied"),
        };
        assert!(io_error.to_string().contains("failed to create"));
        assert!(std::error::Error::source(&io_error).is_some());

        Ok(())
    }
}
