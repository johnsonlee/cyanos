//! Prompt loading and prompt ownership objects.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use crate::TaskLayout;
use crate::r#loop::{InnerPrompt, OuterPrompt};
use crate::terms::{PROGRAM_PROMPT_FILE, PROMPT_FILE, PROMPT_SNAPSHOT_PREFIX};

/// Outer prompt embedded in the executable at build time.
pub const EMBEDDED_OUTER_PROMPT: &str = include_str!("../program.md");

/// Initial coding prompt embedded in the executable at build time.
pub const EMBEDDED_INNER_PROMPT: &str = include_str!("../prompt.md");

/// Loads prompts embedded into the executable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EmbeddedPromptLoader;

impl EmbeddedPromptLoader {
    /// Loads the embedded prompt pair.
    #[must_use]
    pub fn load() -> PromptSet {
        PromptSet::new(
            OuterPrompt::new(EMBEDDED_OUTER_PROMPT.to_owned()),
            InnerPrompt::new(EMBEDDED_INNER_PROMPT.to_owned()),
        )
    }
}

/// Root-relative prompt file paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptPaths {
    program: PathBuf,
    prompt: PathBuf,
}

impl PromptPaths {
    /// Creates prompt paths.
    #[must_use]
    pub fn new(program: PathBuf, prompt: PathBuf) -> Self {
        Self { program, prompt }
    }

    /// Creates prompt paths relative to a repository root.
    #[must_use]
    pub fn from_root(root: &Path) -> Self {
        Self {
            program: root.join(PROGRAM_PROMPT_FILE),
            prompt: root.join(PROMPT_FILE),
        }
    }

    /// Returns the prompt-revision prompt path.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Returns the coding prompt path.
    #[must_use]
    pub fn prompt(&self) -> &Path {
        &self.prompt
    }
}

/// Runtime prompt snapshot name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptSnapshot {
    run_index: usize,
}

impl PromptSnapshot {
    /// Creates a prompt snapshot.
    #[must_use]
    pub const fn new(run_index: usize) -> Self {
        Self { run_index }
    }

    /// Returns the snapshot filename, such as `prompt.2.md`.
    #[must_use]
    pub fn file_name(self) -> String {
        format!("{PROMPT_SNAPSHOT_PREFIX}.{}.md", self.run_index)
    }
}

/// Loaded prompt pair for the two-layer loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptSet {
    outer: OuterPrompt,
    inner: InnerPrompt,
}

impl PromptSet {
    /// Creates a prompt set.
    #[must_use]
    pub fn new(outer: OuterPrompt, inner: InnerPrompt) -> Self {
        Self { outer, inner }
    }

    /// Returns the prompt-revision prompt.
    #[must_use]
    pub const fn outer(&self) -> &OuterPrompt {
        &self.outer
    }

    /// Returns the coding prompt.
    #[must_use]
    pub const fn inner(&self) -> &InnerPrompt {
        &self.inner
    }
}

/// Report produced after writing runtime prompt files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePromptReport {
    program: PathBuf,
    prompt: PathBuf,
    snapshot: PathBuf,
}

impl RuntimePromptReport {
    /// Creates a runtime prompt report.
    #[must_use]
    pub fn new(program: PathBuf, prompt: PathBuf, snapshot: PathBuf) -> Self {
        Self {
            program,
            prompt,
            snapshot,
        }
    }

    /// Returns the runtime prompt-revision prompt path.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Returns the mutable runtime coding prompt path.
    #[must_use]
    pub fn prompt(&self) -> &Path {
        &self.prompt
    }

    /// Returns the immutable prompt snapshot path.
    #[must_use]
    pub fn snapshot(&self) -> &Path {
        &self.snapshot
    }
}

/// Writes embedded prompts into a task runtime directory.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimePromptWriter;

impl RuntimePromptWriter {
    /// Initial prompt snapshot run index.
    pub const INITIAL_SNAPSHOT_INDEX: usize = 0;

    /// Writes `program.md`, `prompt.md`, and `prompt.0.md` for a task.
    ///
    /// # Errors
    ///
    /// Returns an error when any prompt file cannot be written.
    pub fn write_initial(
        task: &TaskLayout,
        prompts: &PromptSet,
    ) -> Result<RuntimePromptReport, PromptWriteError> {
        let program = task.root().join(PROGRAM_PROMPT_FILE);
        let prompt = task.root().join(PROMPT_FILE);
        let snapshot = task.prompt_snapshot(Self::INITIAL_SNAPSHOT_INDEX);

        Self::write(
            &program,
            prompts.outer().as_str(),
            "write runtime prompt-revision prompt",
        )?;
        Self::write(
            &prompt,
            prompts.inner().as_str(),
            "write runtime coding prompt",
        )?;
        Self::write(
            &snapshot,
            prompts.inner().as_str(),
            "write runtime prompt snapshot",
        )?;

        Ok(RuntimePromptReport::new(program, prompt, snapshot))
    }

    fn write(path: &Path, content: &str, action: &'static str) -> Result<(), PromptWriteError> {
        fs::write(path, content).map_err(|source| PromptWriteError {
            action,
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Loads prompts from disk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptLoader {
    paths: PromptPaths,
}

impl PromptLoader {
    /// Creates a prompt loader.
    #[must_use]
    pub fn new(paths: PromptPaths) -> Self {
        Self { paths }
    }

    /// Loads `program.md` as the prompt-revision prompt and `prompt.md` as the coding prompt.
    ///
    /// # Errors
    ///
    /// Returns an error when either prompt file cannot be read.
    pub fn load(&self) -> Result<PromptSet, PromptLoadError> {
        let outer = fs::read_to_string(self.paths.program())
            .map_err(|source| PromptLoadError::new(self.paths.program().to_path_buf(), source))?;
        let inner = fs::read_to_string(self.paths.prompt())
            .map_err(|source| PromptLoadError::new(self.paths.prompt().to_path_buf(), source))?;

        Ok(PromptSet::new(
            OuterPrompt::new(outer),
            InnerPrompt::new(inner),
        ))
    }
}

/// Error returned when a prompt file cannot be loaded.
#[derive(Debug)]
pub struct PromptLoadError {
    path: PathBuf,
    source: io::Error,
}

/// Error returned when runtime prompt files cannot be written.
#[derive(Debug)]
pub struct PromptWriteError {
    action: &'static str,
    path: PathBuf,
    source: io::Error,
}

impl fmt::Display for PromptWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let action = self.action;
        let path = self.path.display();
        write!(f, "failed to {action} at {path}: {}", self.source)
    }
}

impl std::error::Error for PromptWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl PromptLoadError {
    fn new(path: PathBuf, source: io::Error) -> Self {
        Self { path, source }
    }
}

impl fmt::Display for PromptLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.display();
        write!(f, "failed to load prompt from {path}: {}", self.source)
    }
}

impl std::error::Error for PromptLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::{fs, io};

    use super::{
        EmbeddedPromptLoader, PromptLoader, PromptPaths, PromptSet, PromptSnapshot,
        RuntimePromptReport, RuntimePromptWriter,
    };
    use crate::{CyanosHome, InnerPrompt, OuterPrompt, ProjectId, TaskId};

    #[test]
    fn loads_outer_and_inner_prompts_from_repository_root() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!("cyanos-prompts-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        fs::write(root.join("program.md"), "outer program")?;
        fs::write(root.join("prompt.md"), "inner prompt")?;

        let loader = PromptLoader::new(PromptPaths::from_root(&root));
        let prompts = loader
            .load()
            .map_err(|error| io::Error::other(error.to_string()))?;

        assert_eq!(prompts.outer().as_str(), "outer program");
        assert_eq!(prompts.inner().as_str(), "inner prompt");

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn loads_embedded_prompts() {
        let prompts = EmbeddedPromptLoader::load();

        assert!(prompts.outer().as_str().contains("Prompt Revision Task"));
        assert!(prompts.inner().as_str().contains("Coding Task"));
        assert!(prompts.inner().as_str().contains("assigned worktree"));
    }

    #[test]
    fn names_runtime_prompt_snapshots_by_run() {
        assert_eq!(PromptSnapshot::new(3).file_name(), "prompt.3.md");
    }

    #[test]
    fn exposes_prompt_paths_and_prompt_set_fields() {
        let paths = PromptPaths::new("program.md".into(), "prompt.md".into());
        let prompts = super::PromptSet::new(
            super::OuterPrompt::new("outer".to_owned()),
            super::InnerPrompt::new("inner".to_owned()),
        );

        assert_eq!(paths.program(), std::path::Path::new("program.md"));
        assert_eq!(paths.prompt(), std::path::Path::new("prompt.md"));
        assert_eq!(prompts.outer().as_str(), "outer");
        assert_eq!(prompts.inner().as_str(), "inner");
    }

    #[test]
    fn reports_prompt_load_error_with_path_and_source() -> io::Result<()> {
        let root =
            std::env::temp_dir().join(format!("cyanos-missing-prompt-{}", std::process::id()));
        fs::create_dir_all(&root)?;

        let loader = PromptLoader::new(PromptPaths::from_root(&root));
        let result = loader.load();
        assert!(result.is_err());
        let Err(error) = result else {
            fs::remove_dir_all(root)?;
            return Ok(());
        };

        assert!(error.to_string().contains("failed to load prompt from"));
        assert!(error.source().is_some());
        fs::remove_dir_all(root)?;

        Ok(())
    }

    #[test]
    fn writes_initial_runtime_prompt_files() -> Result<(), Box<dyn Error>> {
        let root =
            std::env::temp_dir().join(format!("cyanos-runtime-prompts-{}", std::process::id()));
        let task = CyanosHome::new(root.join("home"))
            .project(ProjectId::new("demo")?)
            .task(TaskId::new("123".to_owned())?);
        fs::create_dir_all(task.root())?;
        let prompts = PromptSet::new(
            OuterPrompt::new("outer runtime".to_owned()),
            InnerPrompt::new("inner runtime".to_owned()),
        );

        let report = RuntimePromptWriter::write_initial(&task, &prompts)?;

        assert_eq!(report.program(), task.root().join("program.md"));
        assert_eq!(report.prompt(), task.root().join("prompt.md"));
        assert_eq!(report.snapshot(), task.root().join("prompt.0.md"));
        assert_eq!(fs::read_to_string(report.program())?, "outer runtime");
        assert_eq!(fs::read_to_string(report.prompt())?, "inner runtime");
        assert_eq!(fs::read_to_string(report.snapshot())?, "inner runtime");

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn exposes_runtime_prompt_report_paths() {
        let report = RuntimePromptReport::new(
            "program.md".into(),
            "prompt.md".into(),
            "prompt.0.md".into(),
        );

        assert_eq!(report.program(), std::path::Path::new("program.md"));
        assert_eq!(report.prompt(), std::path::Path::new("prompt.md"));
        assert_eq!(report.snapshot(), std::path::Path::new("prompt.0.md"));
    }

    #[test]
    fn reports_prompt_write_errors_with_source() -> Result<(), Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!(
            "cyanos-runtime-prompt-missing-{}",
            std::process::id()
        ));
        let task = CyanosHome::new(root.join("home"))
            .project(ProjectId::new("demo")?)
            .task(TaskId::new("123".to_owned())?);
        let prompts = PromptSet::new(
            OuterPrompt::new("outer runtime".to_owned()),
            InnerPrompt::new("inner runtime".to_owned()),
        );

        let result = RuntimePromptWriter::write_initial(&task, &prompts);

        assert!(result.is_err());
        if let Err(error) = result {
            assert!(
                error
                    .to_string()
                    .contains("failed to write runtime prompt-revision prompt")
            );
            assert!(error.source().is_some());
        }

        Ok(())
    }
}
