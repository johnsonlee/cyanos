//! Project initialization for managed Cyanos workspaces.

use std::{
    env, fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::{
    CyanosHome, ProjectId, ProjectLayout, process,
    terms::{CYANOS_DIR, GIT_DIR, GIT_PROGRAM, INIT_ARG, ORIGIN_REMOTE},
};

const DEFAULT_BASE_BRANCH: &str = "main";
const DEFAULT_RUNTIME_BRANCH: &str = "main";
const CONFIG_REPO_KEY: &str = "repo";
const CONFIG_RUNTIME_REPO_KEY: &str = "runtime_repo";
const CONFIG_RUNTIME_REPOSITORY_KEY: &str = "runtime.repository";
const CONFIG_BASE_BRANCH_KEY: &str = "base_branch";
const CONFIG_SOURCE_PATH_KEY: &str = "source_path";
const CONFIG_COMMENT_PREFIX: &str = "#";
const CONFIG_EQUALS_SEPARATOR: char = '=';
const CONFIG_QUOTE: char = '"';
const CONFIG_PARSE_GLOBAL_LINE: usize = 0;
const CONFIG_QUOTED_VALUE_MIN_LEN: usize = 2;
const GITHUB_GIT_SUFFIX: &str = ".git";
const GITHUB_HOST: &str = "github.com";
const GIT_SSH_PREFIX: &str = "git@";
const HTTP_SCHEME: &str = "http";
const HTTPS_SCHEME: &str = "https";
const GITIGNORE_FILE: &str = ".gitignore";
const REMOTE_DEFAULT_BRANCH_REF: &str = "refs/remotes/origin/HEAD";
const REMOTE_BRANCH_PREFIX: &str = "origin/";
const RUNTIME_GIT_EMAIL: &str = "cyanos@example.invalid";
const RUNTIME_GITIGNORE: &str =
    "origin/\ncyanos.lock\ntasks/*/worktree/\ntasks/*/runs/*/samples/*/worktree/\n";

/// Resolves Cyanos runtime home.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HomeResolver;

impl HomeResolver {
    /// Resolves `~/.cyanos`.
    ///
    /// # Errors
    ///
    /// Returns an error when `HOME` is unavailable.
    pub fn resolve() -> Result<CyanosHome, ProjectInitError> {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| CyanosHome::new(home.join(CYANOS_DIR)))
            .ok_or(ProjectInitError::HomeUnavailable)
    }
}

/// Request for `cyanos init`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInitRequest {
    home: CyanosHome,
    project_id: ProjectId,
    source_path: PathBuf,
}

impl ProjectInitRequest {
    /// Creates a project initialization request.
    #[must_use]
    pub fn new(home: CyanosHome, project_id: ProjectId, source_path: PathBuf) -> Self {
        Self {
            home,
            project_id,
            source_path,
        }
    }

    /// Returns the Cyanos runtime home.
    #[must_use]
    pub const fn home(&self) -> &CyanosHome {
        &self.home
    }

    /// Returns the project id.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the source repository path.
    #[must_use]
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }
}

/// Result of project initialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInitReport {
    project: ProjectLayout,
    source_repository: PathBuf,
    wrote_task_readme: bool,
    config: ProjectConfig,
}

impl ProjectInitReport {
    /// Creates an initialization report.
    #[must_use]
    pub const fn new(
        project: ProjectLayout,
        source_repository: PathBuf,
        wrote_task_readme: bool,
        config: ProjectConfig,
    ) -> Self {
        Self {
            project,
            source_repository,
            wrote_task_readme,
            config,
        }
    }

    /// Returns the managed project layout.
    #[must_use]
    pub const fn project(&self) -> &ProjectLayout {
        &self.project
    }

    /// Returns the source repository.
    #[must_use]
    pub fn source_repository(&self) -> &Path {
        &self.source_repository
    }

    /// Returns whether `.cyanos/README.md` was written to the source repo.
    #[must_use]
    pub const fn wrote_task_readme(&self) -> bool {
        self.wrote_task_readme
    }

    /// Returns the persisted project configuration.
    #[must_use]
    pub const fn config(&self) -> &ProjectConfig {
        &self.config
    }
}

/// Typed managed project configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfig {
    repo: String,
    runtime_repo: String,
    base_branch: String,
    source_path: PathBuf,
}

impl ProjectConfig {
    /// Creates project configuration.
    #[must_use]
    pub fn new(
        repo: String,
        runtime_repo: String,
        base_branch: String,
        source_path: PathBuf,
    ) -> Self {
        Self {
            repo,
            runtime_repo,
            base_branch,
            source_path,
        }
    }

    /// Returns the GitHub repository locator for CLI operations.
    #[must_use]
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// Returns the repository locator used for Cyanos runtime evidence.
    #[must_use]
    pub fn runtime_repo(&self) -> &str {
        &self.runtime_repo
    }

    /// Returns the configured base branch.
    #[must_use]
    pub fn base_branch(&self) -> &str {
        &self.base_branch
    }

    /// Returns the source repository path.
    #[must_use]
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// Loads project configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the config file cannot be read or is incomplete.
    pub fn load(project: &ProjectLayout) -> Result<Self, ProjectInitError> {
        let path = project.config();
        let content = fs::read_to_string(&path).map_err(|source| ProjectInitError::Io {
            action: "read project config",
            path: path.clone(),
            source,
        })?;

        Self::parse(&content)
            .map_err(|diagnostic| ProjectInitError::InvalidConfig { path, diagnostic })
    }

    fn parse(content: &str) -> Result<Self, ProjectConfigParseError> {
        let mut repo = None;
        let mut runtime_repo = None;
        let mut base_branch = None;
        let mut source_path = None;

        for (line_index, line) in content.lines().enumerate() {
            let line_number = line_index + 1;
            let line = line.trim();
            if line.is_empty() || line.starts_with(CONFIG_COMMENT_PREFIX) {
                continue;
            }
            let Some((key, value)) = line.split_once(CONFIG_EQUALS_SEPARATOR) else {
                return Err(ProjectConfigParseError::new(
                    line_number,
                    "expected key=value assignment",
                ));
            };
            let key = key.trim();
            let value = parse_config_value(line_number, value)?;
            match key {
                CONFIG_REPO_KEY => set_config_field(&mut repo, key, value, line_number)?,
                CONFIG_RUNTIME_REPO_KEY | CONFIG_RUNTIME_REPOSITORY_KEY => {
                    set_config_field(&mut runtime_repo, key, value, line_number)?;
                }
                CONFIG_BASE_BRANCH_KEY => {
                    set_config_field(&mut base_branch, key, value, line_number)?;
                }
                CONFIG_SOURCE_PATH_KEY => {
                    set_config_field(&mut source_path, key, PathBuf::from(value), line_number)?;
                }
                _ => {
                    return Err(ProjectConfigParseError::new(
                        line_number,
                        format!("unknown project config key `{key}`"),
                    ));
                }
            }
        }

        let repo = required_config_field(repo, CONFIG_REPO_KEY)?;
        let runtime_repo = runtime_repo.unwrap_or_else(|| derive_runtime_repo(&repo));
        Ok(Self::new(
            repo,
            runtime_repo,
            required_config_field(base_branch, CONFIG_BASE_BRANCH_KEY)?,
            required_config_field(source_path, CONFIG_SOURCE_PATH_KEY)?,
        ))
    }

    fn write(&self, project: &ProjectLayout) -> Result<(), ProjectInitError> {
        let content = format!(
            "repo={}\nruntime_repo={}\nbase_branch={}\nsource_path={}\n",
            self.repo(),
            self.runtime_repo(),
            self.base_branch(),
            self.source_path().display()
        );
        fs::write(project.config(), content).map_err(|source| ProjectInitError::Io {
            action: "write project config",
            path: project.config(),
            source,
        })
    }
}

fn parse_config_value(line_number: usize, value: &str) -> Result<String, ProjectConfigParseError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ProjectConfigParseError::new(
            line_number,
            "expected non-empty value",
        ));
    }

    let starts_quoted = value.starts_with(CONFIG_QUOTE);
    let ends_quoted = value.ends_with(CONFIG_QUOTE);
    match (starts_quoted, ends_quoted) {
        (true, true) if value.len() >= CONFIG_QUOTED_VALUE_MIN_LEN => {
            Ok(value[1..value.len() - 1].to_owned())
        }
        (false, false) => Ok(value.to_owned()),
        _ => Err(ProjectConfigParseError::new(
            line_number,
            "unbalanced quoted value",
        )),
    }
}

fn set_config_field<T>(
    target: &mut Option<T>,
    key: &str,
    value: T,
    line_number: usize,
) -> Result<(), ProjectConfigParseError> {
    if target.is_some() {
        return Err(ProjectConfigParseError::new(
            line_number,
            format!("duplicate project config key `{key}`"),
        ));
    }
    *target = Some(value);
    Ok(())
}

fn required_config_field<T>(
    value: Option<T>,
    key: &'static str,
) -> Result<T, ProjectConfigParseError> {
    value.ok_or_else(|| {
        ProjectConfigParseError::new(
            CONFIG_PARSE_GLOBAL_LINE,
            format!("missing required project config key `{key}`"),
        )
    })
}

/// Diagnostic returned when managed project configuration cannot be parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfigParseError {
    line: usize,
    message: String,
}

impl ProjectConfigParseError {
    /// Creates a project config parser diagnostic.
    #[must_use]
    pub fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }

    /// Returns the one-based config line, or zero for file-level diagnostics.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the parser diagnostic message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ProjectConfigParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line() == CONFIG_PARSE_GLOBAL_LINE {
            f.write_str(self.message())
        } else {
            write!(f, "line {}: {}", self.line(), self.message())
        }
    }
}

impl std::error::Error for ProjectConfigParseError {}

/// Git command runner.
pub trait GitRunner {
    /// Runs `git` with args in the provided directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be launched or exits
    /// unsuccessfully.
    fn run(&self, cwd: &Path, args: &[&str]) -> Result<(), ProjectInitError>;
}

impl<T> GitRunner for &T
where
    T: GitRunner,
{
    fn run(&self, cwd: &Path, args: &[&str]) -> Result<(), ProjectInitError> {
        (*self).run(cwd, args)
    }
}

/// System `git` command runner.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemGitRunner;

impl GitRunner for SystemGitRunner {
    fn run(&self, cwd: &Path, args: &[&str]) -> Result<(), ProjectInitError> {
        let output = process::output(GIT_PROGRAM, cwd, args).map_err(|source| {
            ProjectInitError::GitSpawn {
                cwd: cwd.to_path_buf(),
                source,
            }
        })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(ProjectInitError::GitFailed {
                cwd: cwd.to_path_buf(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                code: output.status.code(),
            })
        }
    }
}

/// Initializes a managed Cyanos project.
#[derive(Clone, Debug)]
pub struct ProjectInitializer<G> {
    git: G,
}

impl<G> ProjectInitializer<G>
where
    G: GitRunner,
{
    /// Creates a project initializer.
    #[must_use]
    pub const fn new(git: G) -> Self {
        Self { git }
    }

    /// Initializes a project.
    ///
    /// # Errors
    ///
    /// Returns an error when directories or git repository setup fail.
    pub fn initialize(
        &self,
        request: &ProjectInitRequest,
    ) -> Result<ProjectInitReport, ProjectInitError> {
        let source = request.source_path().to_path_buf();
        let project = request.home().project(request.project_id().clone());
        let target = ResolvedRepository::parse(request.project_id().as_str()).ok_or_else(|| {
            ProjectInitError::InvalidProjectLocator(request.project_id().as_str().to_owned())
        })?;

        if !source.exists() {
            return Err(ProjectInitError::SourceMissing(source));
        }

        if !source.is_dir() {
            return Err(ProjectInitError::SourceNotDirectory(source));
        }

        if project.root().exists() {
            return Err(ProjectInitError::ProjectAlreadyExists(project.root()));
        }

        let source_origin = infer_github_remote(&source);
        if let Some(origin) = source_origin.as_deref() {
            let Some(actual) = ResolvedRepository::parse(origin) else {
                return Err(ProjectInitError::OriginRemoteMismatch {
                    expected: target.repo().to_owned(),
                    actual: origin.to_owned(),
                });
            };
            if actual.repo() != target.repo() {
                return Err(ProjectInitError::OriginRemoteMismatch {
                    expected: target.repo().to_owned(),
                    actual: actual.repo().to_owned(),
                });
            }
        }

        Self::create_dir(&project.root(), "create project directory")?;
        Self::create_dir(&project.tasks(), "create task directory")?;

        let source_is_git = source.join(GIT_DIR).exists();
        let config = ProjectConfig::new(
            target.repo().to_owned(),
            target.runtime_repo(),
            infer_base_branch(&source).unwrap_or_else(|| DEFAULT_BASE_BRANCH.to_owned()),
            source.clone(),
        );

        self.initialize_runtime_repository(&project)?;

        if source_is_git {
            let origin_parent = project
                .origin()
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| ProjectInitError::InvalidPath(project.origin()))?;
            Self::create_dir(&origin_parent, "create origin parent directory")?;
            let origin = project.origin();
            let origin_arg = origin.to_string_lossy().to_string();
            let source_arg = source.to_string_lossy().to_string();
            self.git.run(
                Path::new("."),
                &["clone", source_arg.as_str(), origin_arg.as_str()],
            )?;
            self.git.run(
                &project.origin(),
                &["remote", "set-url", ORIGIN_REMOTE, target.remote_url()],
            )?;
        } else {
            Self::create_dir(&project.origin(), "create origin repository directory")?;
            self.git.run(&project.origin(), &[INIT_ARG])?;
        }
        config.write(&project)?;

        Ok(ProjectInitReport::new(project, source, false, config))
    }

    fn initialize_runtime_repository(
        &self,
        project: &ProjectLayout,
    ) -> Result<(), ProjectInitError> {
        self.git.run(project.root().as_path(), &[INIT_ARG])?;
        self.git.run(
            project.root().as_path(),
            &["checkout", "-B", DEFAULT_RUNTIME_BRANCH],
        )?;
        self.git
            .run(project.root().as_path(), &["config", "user.name", "Cyanos"])?;
        self.git.run(
            project.root().as_path(),
            &["config", "user.email", RUNTIME_GIT_EMAIL],
        )?;
        fs::write(project.root().join(GITIGNORE_FILE), RUNTIME_GITIGNORE).map_err(|source| {
            ProjectInitError::Io {
                action: "write runtime gitignore",
                path: project.root().join(GITIGNORE_FILE),
                source,
            }
        })
    }

    fn create_dir(path: &Path, action: &'static str) -> Result<(), ProjectInitError> {
        fs::create_dir_all(path).map_err(|source| ProjectInitError::Io {
            action,
            path: path.to_path_buf(),
            source,
        })
    }
}

fn infer_base_branch(source: &Path) -> Option<String> {
    if let Some(branch) = infer_remote_default_branch(source) {
        return Some(branch);
    }

    let output =
        process::output(GIT_PROGRAM, source, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch)
    }
}

fn infer_remote_default_branch(source: &Path) -> Option<String> {
    let output = process::output(
        GIT_PROGRAM,
        source,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            REMOTE_DEFAULT_BRANCH_REF,
        ],
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    branch
        .strip_prefix(REMOTE_BRANCH_PREFIX)
        .filter(|branch| !branch.is_empty())
        .map(str::to_owned)
}

fn infer_github_remote(source: &Path) -> Option<String> {
    let output =
        process::output(GIT_PROGRAM, source, &["remote", "get-url", ORIGIN_REMOTE]).ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn derive_runtime_repo(repo: &str) -> String {
    let mut parts = repo.split('/');
    let Some(first) = parts.next() else {
        return String::new();
    };
    let Some(second) = parts.next() else {
        return String::new();
    };
    let third = parts.next();
    if parts.next().is_some() {
        return String::new();
    }

    match third {
        Some(name)
            if valid_repo_component(first)
                && valid_repo_component(second)
                && valid_repo_component(name) =>
        {
            format!("{first}/{second}/cyanos-{name}")
        }
        None if valid_repo_component(first) && valid_repo_component(second) => {
            format!("{first}/cyanos-{second}")
        }
        Some(_) | None => String::new(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedRepository {
    repo: String,
    remote_url: String,
}

impl ResolvedRepository {
    fn parse(value: &str) -> Option<Self> {
        let repo = normalize_repository_locator(value)?;
        let remote_url = canonical_repository_remote(&repo);
        Some(Self { repo, remote_url })
    }

    fn repo(&self) -> &str {
        &self.repo
    }

    fn runtime_repo(&self) -> String {
        derive_runtime_repo(&self.repo)
    }

    fn remote_url(&self) -> &str {
        &self.remote_url
    }
}

fn normalize_repository_locator(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(GITHUB_GIT_SUFFIX);
    if let Some(rest) = value.strip_prefix(GIT_SSH_PREFIX) {
        return normalize_ssh_repository(rest);
    }

    if let Some((scheme, rest)) = value.split_once("://") {
        return normalize_url_repository(scheme, rest);
    }

    normalize_owner_repo(value)
}

fn normalize_ssh_repository(rest: &str) -> Option<String> {
    let (host, path) = rest.split_once(':')?;
    normalize_host_path_repository(host, path)
}

fn normalize_url_repository(scheme: &str, rest: &str) -> Option<String> {
    let scheme = scheme.to_ascii_lowercase();
    if scheme != HTTPS_SCHEME && scheme != HTTP_SCHEME {
        return None;
    }

    let (host, path) = rest.split_once('/')?;
    normalize_host_path_repository(host, path)
}

fn normalize_host_path_repository(host: &str, path: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    if !valid_host(&host) {
        return None;
    }

    let repo = normalize_owner_repo(path)?;
    if host == GITHUB_HOST {
        Some(repo)
    } else {
        Some(format!("{host}/{repo}"))
    }
}

fn normalize_owner_repo(path: &str) -> Option<String> {
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next().is_some() || !valid_repo_component(owner) || !valid_repo_component(repo) {
        None
    } else {
        Some(format!("{owner}/{repo}"))
    }
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

fn canonical_repository_remote(repo: &str) -> String {
    let mut parts = repo.split('/');
    let Some(first) = parts.next() else {
        return format!("https://{GITHUB_HOST}/{repo}{GITHUB_GIT_SUFFIX}");
    };
    let Some(second) = parts.next() else {
        return format!("https://{GITHUB_HOST}/{repo}{GITHUB_GIT_SUFFIX}");
    };
    match (parts.next(), parts.next()) {
        (Some(third), None) => format!("https://{first}/{second}/{third}{GITHUB_GIT_SUFFIX}"),
        (None, None) => {
            format!("https://{GITHUB_HOST}/{first}/{second}{GITHUB_GIT_SUFFIX}")
        }
        _ => format!("https://{GITHUB_HOST}/{repo}{GITHUB_GIT_SUFFIX}"),
    }
}

/// Error returned by project initialization.
#[derive(Debug)]
pub enum ProjectInitError {
    /// HOME could not be resolved.
    HomeUnavailable,
    /// Project directory already exists.
    ProjectAlreadyExists(PathBuf),
    /// Source path does not exist.
    SourceMissing(PathBuf),
    /// Source path exists but is not a directory.
    SourceNotDirectory(PathBuf),
    /// A derived path was invalid.
    InvalidPath(PathBuf),
    /// Project configuration is invalid.
    InvalidConfig {
        /// Configuration file path.
        path: PathBuf,
        /// Parser diagnostic.
        diagnostic: ProjectConfigParseError,
    },
    /// The project locator cannot be resolved to a target repository.
    InvalidProjectLocator(String),
    /// The current repository remote does not match the requested project.
    OriginRemoteMismatch {
        /// Repository requested by `--project`.
        expected: String,
        /// Repository resolved from the current repository remote.
        actual: String,
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

impl fmt::Display for ProjectInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeUnavailable => f.write_str("HOME is not available"),
            Self::ProjectAlreadyExists(path) => {
                write!(f, "project already exists at {}", path.display())
            }
            Self::SourceMissing(path) => {
                write!(f, "source path does not exist at {}", path.display())
            }
            Self::SourceNotDirectory(path) => {
                write!(f, "source path is not a directory at {}", path.display())
            }
            Self::InvalidPath(path) => write!(f, "invalid path: {}", path.display()),
            Self::InvalidConfig { path, diagnostic } => {
                write!(
                    f,
                    "project config is invalid at {}: {diagnostic}",
                    path.display()
                )
            }
            Self::InvalidProjectLocator(locator) => {
                write!(
                    f,
                    "invalid project locator: {locator}; use owner/repo or a full repository URL"
                )
            }
            Self::OriginRemoteMismatch { expected, actual } => {
                write!(
                    f,
                    "current origin remote resolves to {actual}, but --project resolves to {expected}"
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

impl std::error::Error for ProjectInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::GitSpawn { source, .. } => Some(source),
            Self::HomeUnavailable
            | Self::ProjectAlreadyExists(_)
            | Self::SourceMissing(_)
            | Self::SourceNotDirectory(_)
            | Self::InvalidPath(_)
            | Self::InvalidConfig { .. }
            | Self::InvalidProjectLocator(_)
            | Self::OriginRemoteMismatch { .. }
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

    use super::{canonical_repository_remote, derive_runtime_repo, normalize_repository_locator};
    use crate::{
        CyanosHome, GitRunner, ProjectConfig, ProjectConfigParseError, ProjectId, ProjectInitError,
        ProjectInitRequest, ProjectInitializer,
    };

    #[derive(Debug, Default)]
    struct RecordingGitRunner {
        calls: RefCell<Vec<(PathBuf, Vec<String>)>>,
        fail: bool,
    }

    impl RecordingGitRunner {
        fn calls(&self) -> Vec<(PathBuf, Vec<String>)> {
            self.calls.borrow().clone()
        }
    }

    impl GitRunner for RecordingGitRunner {
        fn run(&self, cwd: &Path, args: &[&str]) -> Result<(), ProjectInitError> {
            self.calls.borrow_mut().push((
                cwd.to_path_buf(),
                args.iter().map(|arg| (*arg).to_owned()).collect(),
            ));
            if self.fail {
                Err(ProjectInitError::GitFailed {
                    cwd: cwd.to_path_buf(),
                    args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                    code: Some(1),
                })
            } else {
                Ok(())
            }
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("cyanos-{name}-{}", std::process::id()))
    }

    fn run_git(cwd: &Path, args: &[&str]) -> io::Result<()> {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "git {} failed with {:?}",
                args.join(" "),
                output.status.code()
            )))
        }
    }

    #[test]
    fn initializes_non_git_project_origin() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("init-non-git");
        let source = root.join("source");
        fs::create_dir_all(&source)?;
        let git = RecordingGitRunner::default();
        let initializer = ProjectInitializer::new(&git);
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source.clone(),
        );

        let report = initializer.initialize(&request)?;

        assert!(report.project().origin().is_dir());
        assert_eq!(report.source_repository(), source);
        assert!(!report.wrote_task_readme());
        assert_eq!(ProjectConfig::load(report.project())?, *report.config());
        assert_eq!(report.config().repo(), "owner/repo");
        assert_eq!(report.config().runtime_repo(), "owner/cyanos-repo");
        assert!(report.project().root().join(".gitignore").is_file());
        assert!(
            git.calls()
                .iter()
                .any(|call| call.0 == report.project().root() && call.1 == ["init".to_owned()])
        );
        assert!(
            git.calls()
                .iter()
                .any(|call| call.0 == report.project().origin() && call.1 == ["init".to_owned()])
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn initializes_git_project_by_cloning_source_and_writing_config()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("init-git");
        let source = root.join("source");
        fs::create_dir_all(source.join(".git"))?;
        let git = RecordingGitRunner::default();
        let initializer = ProjectInitializer::new(&git);
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source.clone(),
        );

        let report = initializer.initialize(&request)?;

        assert!(!source.join(".cyanos/README.md").exists());
        assert!(report.project().config().is_file());
        assert!(!report.wrote_task_readme());
        assert_eq!(report.config().repo(), "owner/repo");
        assert_eq!(report.config().base_branch(), "main");
        assert_eq!(report.config().source_path(), source);
        assert!(
            git.calls()
                .iter()
                .any(|call| call.1.first().map(String::as_str) == Some("clone"))
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn leaves_existing_task_readme_in_source_only_during_git_project_init()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("init-existing-readme");
        let source = root.join("source");
        fs::create_dir_all(source.join(".git"))?;
        fs::create_dir_all(source.join(".cyanos"))?;
        fs::write(
            source.join(".cyanos/README.md"),
            "Task ID: 123\n\nKeep existing instructions.\n",
        )?;
        let git = RecordingGitRunner::default();
        let initializer = ProjectInitializer::new(&git);
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source.clone(),
        );

        let report = initializer.initialize(&request)?;

        assert!(!report.wrote_task_readme());
        assert_eq!(
            fs::read_to_string(source.join(".cyanos/README.md"))?,
            "Task ID: 123\n\nKeep existing instructions.\n"
        );
        assert!(!report.project().origin().join(".cyanos/README.md").exists());
        assert!(report.project().config().is_file());

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn recognizes_git_worktree_file_marker() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("init-worktree-file");
        let source = root.join("source");
        fs::create_dir_all(&source)?;
        fs::write(source.join(".git"), "gitdir: ../actual.git\n")?;
        let git = RecordingGitRunner::default();
        let initializer = ProjectInitializer::new(&git);
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source,
        );

        let report = initializer.initialize(&request)?;

        assert!(!report.wrote_task_readme());
        assert!(report.project().config().is_file());
        assert!(
            git.calls()
                .iter()
                .any(|call| call.1.first().map(String::as_str) == Some("clone"))
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn stores_project_locator_repository_config() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("init-explicit-repo");
        let source = root.join("source");
        fs::create_dir_all(&source)?;
        let git = RecordingGitRunner::default();
        let initializer = ProjectInitializer::new(&git);
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source,
        );

        let report = initializer.initialize(&request)?;

        assert_eq!(report.config().repo(), "owner/repo");
        assert_eq!(ProjectConfig::load(report.project())?.repo(), "owner/repo");

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn loads_config_with_strict_diagnostics_and_runtime_override()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("config-load");
        let project = CyanosHome::new(root.join("home")).project(ProjectId::new("demo")?);

        let missing = ProjectConfig::load(&project);
        assert!(matches!(missing, Err(ProjectInitError::Io { .. })));

        fs::create_dir_all(project.root())?;
        fs::write(
            project.config(),
            "repo=owner/repo\nbase_branch=main\nsource_path=/tmp/source\n",
        )?;
        let config = ProjectConfig::load(&project)?;
        assert_eq!(config.repo(), "owner/repo");
        assert_eq!(config.runtime_repo(), "owner/cyanos-repo");
        assert_eq!(config.base_branch(), "main");
        fs::write(
            project.config(),
            "repo=owner/repo\nruntime.repository = \"owner/custom-runtime\"\nbase_branch=main\nsource_path=/tmp/source\n",
        )?;
        let config = ProjectConfig::load(&project)?;
        assert_eq!(config.runtime_repo(), "owner/custom-runtime");

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_malformed_project_config_lines() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("config-strict");
        let project = CyanosHome::new(root.join("home")).project(ProjectId::new("demo")?);
        fs::create_dir_all(project.root())?;

        fs::write(
            project.config(),
            "\n# managed config\nrepo=owner/repo\nbase_branch=main\n",
        )?;
        let invalid = ProjectConfig::load(&project);
        assert!(matches!(
            invalid,
            Err(ProjectInitError::InvalidConfig { diagnostic, .. })
                if diagnostic.message().contains("missing required project config key")
                    && diagnostic.line() == 0
        ));
        fs::write(
            project.config(),
            "repo=owner/repo\nbase_branch=\nsource_path=/tmp/source\n",
        )?;
        let invalid = ProjectConfig::load(&project);
        assert!(matches!(
            invalid,
            Err(ProjectInitError::InvalidConfig { diagnostic, .. })
                if diagnostic.message() == "expected non-empty value"
        ));
        fs::write(project.config(), "not a key value\nrepo=owner/repo\n")?;
        let invalid = ProjectConfig::load(&project);
        assert!(matches!(
            invalid,
            Err(ProjectInitError::InvalidConfig { diagnostic, .. })
                if diagnostic == ProjectConfigParseError::new(1, "expected key=value assignment")
        ));
        fs::write(
            project.config(),
            "repo=owner/repo\nruntime.repositroy=owner/runtime\nbase_branch=main\nsource_path=/tmp/source\n",
        )?;
        let invalid = ProjectConfig::load(&project);
        assert!(matches!(
            invalid,
            Err(ProjectInitError::InvalidConfig { diagnostic, .. })
                if diagnostic.message().contains("unknown project config key")
        ));
        fs::write(
            project.config(),
            "repo=owner/repo\nrepo=owner/other\nbase_branch=main\nsource_path=/tmp/source\n",
        )?;
        let invalid = ProjectConfig::load(&project);
        assert!(matches!(
            invalid,
            Err(ProjectInitError::InvalidConfig { diagnostic, .. })
                if diagnostic.message().contains("duplicate project config key")
        ));
        fs::write(
            project.config(),
            "repo=\"owner/repo\nbase_branch=main\nsource_path=/tmp/source\n",
        )?;
        let invalid = ProjectConfig::load(&project);
        assert!(matches!(
            invalid,
            Err(ProjectInitError::InvalidConfig { diagnostic, .. })
                if diagnostic.message() == "unbalanced quoted value"
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn formats_project_config_parser_diagnostics() {
        assert_eq!(
            ProjectConfigParseError::new(0, "missing repo").to_string(),
            "missing repo"
        );
        assert_eq!(
            ProjectConfigParseError::new(7, "unknown key").to_string(),
            "line 7: unknown key"
        );
    }

    #[test]
    fn infers_repository_config_from_source_git_remote() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("infer-source-remote");
        let source = root.join("source");
        fs::create_dir_all(&source)?;
        run_git(&source, &["init"])?;
        run_git(&source, &["checkout", "-B", "feature"])?;
        run_git(&source, &["config", "user.name", "Cyanos Test"])?;
        run_git(
            &source,
            &["config", "user.email", "cyanos-test@example.com"],
        )?;
        fs::write(source.join("README.md"), "# Source\n")?;
        run_git(&source, &["add", "README.md"])?;
        run_git(&source, &["commit", "-m", "Initial commit"])?;
        run_git(
            &source,
            &["remote", "add", "origin", "git@github.com:owner/repo.git"],
        )?;
        let git = RecordingGitRunner::default();
        let initializer = ProjectInitializer::new(&git);
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source,
        );

        let report = initializer.initialize(&request)?;

        assert_eq!(report.config().repo(), "owner/repo");
        assert_eq!(report.config().base_branch(), "feature");
        assert!(git.calls().iter().any(|call| {
            call.0 == report.project().origin()
                && call.1
                    == [
                        "remote".to_owned(),
                        "set-url".to_owned(),
                        "origin".to_owned(),
                        "https://github.com/owner/repo.git".to_owned(),
                    ]
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_origin_remote_that_does_not_match_project_locator()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("origin-mismatch");
        let source = root.join("source");
        fs::create_dir_all(&source)?;
        run_git(&source, &["init"])?;
        run_git(
            &source,
            &["remote", "add", "origin", "git@github.com:other/repo.git"],
        )?;
        let initializer = ProjectInitializer::new(RecordingGitRunner::default());
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source,
        );

        let result = initializer.initialize(&request);

        assert!(matches!(
            result,
            Err(ProjectInitError::OriginRemoteMismatch { .. })
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_origin_remote_that_cannot_be_normalized() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("origin-unparseable");
        let source = root.join("source");
        fs::create_dir_all(&source)?;
        run_git(&source, &["init"])?;
        run_git(&source, &["remote", "add", "origin", "file:///tmp/repo"])?;
        let initializer = ProjectInitializer::new(RecordingGitRunner::default());
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source,
        );

        let result = initializer.initialize(&request);

        assert!(matches!(
            result,
            Err(ProjectInitError::OriginRemoteMismatch { .. })
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn infers_remote_default_branch_before_local_head() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("infer-remote-head");
        let source = root.join("source");
        fs::create_dir_all(&source)?;
        run_git(&source, &["init"])?;
        run_git(&source, &["checkout", "-B", "local"])?;
        run_git(
            &source,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        assert_eq!(super::infer_base_branch(&source).as_deref(), Some("main"));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rewrites_managed_clone_remote_for_github_projects() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = temp_root("init-github-remote");
        let source = root.join("source");
        fs::create_dir_all(source.join(".git"))?;
        let git = RecordingGitRunner::default();
        let initializer = ProjectInitializer::new(&git);
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source,
        );

        let report = initializer.initialize(&request)?;
        let calls = git.calls();

        assert_eq!(report.config().repo(), "owner/repo");
        assert!(calls.iter().any(|call| {
            call.0 == report.project().origin()
                && call.1
                    == [
                        "remote".to_owned(),
                        "set-url".to_owned(),
                        "origin".to_owned(),
                        "https://github.com/owner/repo.git".to_owned(),
                    ]
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn normalizes_repository_locator_forms() {
        assert_eq!(
            normalize_repository_locator("git@github.com:owner/repo.git").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            normalize_repository_locator("git@github.example.com:owner/repo.git").as_deref(),
            Some("github.example.com/owner/repo")
        );
        assert_eq!(
            normalize_repository_locator("https://github.com/owner/repo").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            normalize_repository_locator("https://github.example.com/owner/repo").as_deref(),
            Some("github.example.com/owner/repo")
        );
        assert_eq!(
            normalize_repository_locator("https://github.com/owner/repo/extra"),
            None
        );
        assert_eq!(
            normalize_repository_locator("https://github.com//repo"),
            None
        );
        assert_eq!(
            normalize_repository_locator("https://github.com/owner/"),
            None
        );
        assert_eq!(
            normalize_repository_locator("ssh://github.com/owner/repo"),
            None
        );
        assert_eq!(
            normalize_repository_locator("https://bad_host!/owner/repo"),
            None
        );
        assert_eq!(normalize_repository_locator("owner/repo/extra"), None);
        assert_eq!(normalize_repository_locator(".owner/repo"), None);
        assert_eq!(normalize_repository_locator("owner/.repo"), None);
        assert_eq!(
            canonical_repository_remote("owner/repo"),
            "https://github.com/owner/repo.git"
        );
        assert_eq!(
            canonical_repository_remote("github.example.com/owner/repo"),
            "https://github.example.com/owner/repo.git"
        );
        assert_eq!(
            canonical_repository_remote("owner"),
            "https://github.com/owner.git"
        );
        assert_eq!(
            canonical_repository_remote("host/owner/repo/extra"),
            "https://github.com/host/owner/repo/extra.git"
        );
    }

    #[test]
    fn derives_runtime_repositories_from_normalized_targets() {
        assert_eq!(derive_runtime_repo("owner/repo"), "owner/cyanos-repo");
        assert_eq!(
            derive_runtime_repo("github.example.com/owner/repo"),
            "github.example.com/owner/cyanos-repo"
        );
        assert!(derive_runtime_repo("owner").is_empty());
        assert!(derive_runtime_repo("owner/.repo").is_empty());
        assert!(derive_runtime_repo("host/owner/.repo").is_empty());
        assert!(derive_runtime_repo("host/owner/repo/extra").is_empty());
    }

    #[test]
    fn rejects_existing_project() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("init-existing");
        let source = root.join("source");
        let home = CyanosHome::new(root.join("home"));
        let project_id = ProjectId::new("owner/repo")?;
        fs::create_dir_all(home.project(project_id.clone()).root())?;
        fs::create_dir_all(&source)?;
        let initializer = ProjectInitializer::new(RecordingGitRunner::default());
        let request = ProjectInitRequest::new(home, project_id, source);
        let result = initializer.initialize(&request);

        assert!(matches!(
            result,
            Err(ProjectInitError::ProjectAlreadyExists(_))
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_project_ids_that_are_not_repository_locators()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("invalid-project-locator");
        let source = root.join("source");
        fs::create_dir_all(&source)?;
        let initializer = ProjectInitializer::new(RecordingGitRunner::default());
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("demo")?,
            source,
        );

        let result = initializer.initialize(&request);

        assert!(matches!(
            result,
            Err(ProjectInitError::InvalidProjectLocator(_))
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_missing_source_path() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("missing-source");
        let initializer = ProjectInitializer::new(RecordingGitRunner::default());
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            root.join("missing"),
        );
        let result = initializer.initialize(&request);

        assert!(matches!(result, Err(ProjectInitError::SourceMissing(_))));

        Ok(())
    }

    #[test]
    fn rejects_file_source_path() -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("file-source");
        fs::create_dir_all(&root)?;
        let source = root.join("source.txt");
        fs::write(&source, "not a directory")?;
        let initializer = ProjectInitializer::new(RecordingGitRunner::default());
        let request = ProjectInitRequest::new(
            CyanosHome::new(root.join("home")),
            ProjectId::new("owner/repo")?,
            source,
        );
        let result = initializer.initialize(&request);

        assert!(matches!(
            result,
            Err(ProjectInitError::SourceNotDirectory(_))
        ));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn displays_and_sources_init_errors() {
        let io_error = ProjectInitError::Io {
            action: "create",
            path: PathBuf::from("/tmp/demo"),
            source: io::Error::other("denied"),
        };
        let git_spawn = ProjectInitError::GitSpawn {
            cwd: PathBuf::from("/tmp/demo"),
            source: io::Error::other("missing"),
        };

        assert_eq!(
            ProjectInitError::HomeUnavailable.to_string(),
            "HOME is not available"
        );
        assert!(
            ProjectInitError::ProjectAlreadyExists(PathBuf::from("/tmp/demo"))
                .to_string()
                .contains("project already exists")
        );
        assert!(
            ProjectInitError::SourceMissing(PathBuf::from("/tmp/demo"))
                .to_string()
                .contains("source path does not exist")
        );
        assert!(
            ProjectInitError::SourceNotDirectory(PathBuf::from("/tmp/demo"))
                .to_string()
                .contains("source path is not a directory")
        );
        assert!(
            ProjectInitError::InvalidPath(PathBuf::from("/tmp/demo"))
                .to_string()
                .contains("invalid path")
        );
        assert!(io_error.to_string().contains("failed to create"));
        assert!(git_spawn.to_string().contains("failed to launch git"));
        assert!(
            ProjectInitError::GitFailed {
                cwd: PathBuf::from("/tmp/demo"),
                args: vec!["init".to_owned()],
                code: Some(1),
            }
            .to_string()
            .contains("git init failed")
        );
        assert!(std::error::Error::source(&io_error).is_some());
        assert!(std::error::Error::source(&git_spawn).is_some());
        assert!(
            ProjectInitError::InvalidConfig {
                path: PathBuf::from("/tmp/config.txt"),
                diagnostic: ProjectConfigParseError::new(1, "unknown project config key `x`"),
            }
            .to_string()
            .contains("project config is invalid")
        );
        assert!(
            std::error::Error::source(&ProjectInitError::InvalidConfig {
                path: PathBuf::from("/tmp/config.txt"),
                diagnostic: ProjectConfigParseError::new(1, "unknown project config key `x`"),
            })
            .is_none()
        );
    }

    #[test]
    fn system_git_runner_reports_project_init_command_results()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("system-project-git");
        fs::create_dir_all(&root)?;
        let git = super::SystemGitRunner;

        git.run(&root, &["--version"])?;
        let result = git.run(&root, &["definitely-not-a-cyanos-command"]);
        assert!(matches!(result, Err(ProjectInitError::GitFailed { .. })));
        let spawn = git.run(&root.join("missing"), &["--version"]);
        assert!(matches!(spawn, Err(ProjectInitError::GitSpawn { .. })));

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
