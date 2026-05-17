//! Dependency requirements checked before running an agent.

use std::{fmt, io, path::Path};

use crate::{
    AdapterRegistry, Agent, CommandSpec, process,
    terms::{
        AUTH_ARG, CARGO_PROGRAM, CLI_FLAG_VERSION, GIT_PROGRAM, GITHUB_CLI_PROGRAM, STATUS_ARG,
    },
};

const DEPENDENCY_RETRY_ATTEMPTS: usize = 3;

/// Dependency check required before `cyanos run`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyRequirement {
    /// `git` must be installed.
    GitInstalled,
    /// `cargo` must be installed for the Rust verifier.
    CargoInstalled,
    /// `gh` must be installed.
    GitHubCliInstalled,
    /// `gh` must already be authenticated.
    GitHubCliLoggedIn,
    /// The selected agent CLI must be installed.
    AgentCliInstalled(Agent),
    /// The selected agent CLI must already be authenticated.
    AgentCliLoggedIn(Agent),
}

impl DependencyRequirement {
    /// Returns a concise user-facing dependency name.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::GitInstalled => "git installed".to_owned(),
            Self::CargoInstalled => "cargo installed".to_owned(),
            Self::GitHubCliInstalled => "gh installed".to_owned(),
            Self::GitHubCliLoggedIn => "gh logged in".to_owned(),
            Self::AgentCliInstalled(agent) => format!("{} installed", agent.as_str()),
            Self::AgentCliLoggedIn(agent) => format!("{} logged in", agent.as_str()),
        }
    }
}

/// Ordered dependency check plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyPlan {
    requirements: Vec<DependencyRequirement>,
}

impl DependencyPlan {
    /// Creates a dependency plan.
    #[must_use]
    pub fn new(requirements: Vec<DependencyRequirement>) -> Self {
        Self { requirements }
    }

    /// Creates the standard `run` dependency plan for the worker and judge agents.
    #[must_use]
    pub fn for_run(worker_agent: Agent, judge_agent: Agent) -> Self {
        let mut requirements = vec![
            DependencyRequirement::GitInstalled,
            DependencyRequirement::CargoInstalled,
            DependencyRequirement::GitHubCliInstalled,
            DependencyRequirement::GitHubCliLoggedIn,
            DependencyRequirement::AgentCliInstalled(worker_agent),
            DependencyRequirement::AgentCliLoggedIn(worker_agent),
        ];

        if judge_agent != worker_agent {
            requirements.push(DependencyRequirement::AgentCliInstalled(judge_agent));
            requirements.push(DependencyRequirement::AgentCliLoggedIn(judge_agent));
        }

        Self::new(requirements)
    }

    /// Returns all required checks.
    #[must_use]
    pub fn requirements(&self) -> &[DependencyRequirement] {
        &self.requirements
    }
}

/// Result for one dependency check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyCheck {
    requirement: DependencyRequirement,
    command: Vec<String>,
    attempts: usize,
}

impl DependencyCheck {
    /// Creates a dependency check result.
    #[must_use]
    pub fn new(requirement: DependencyRequirement, command: Vec<String>) -> Self {
        Self {
            requirement,
            command,
            attempts: 1,
        }
    }

    /// Creates a dependency check result with the number of attempts used.
    #[must_use]
    pub fn new_with_attempts(
        requirement: DependencyRequirement,
        command: Vec<String>,
        attempts: usize,
    ) -> Self {
        Self {
            requirement,
            command,
            attempts,
        }
    }

    /// Returns the checked requirement.
    #[must_use]
    pub const fn requirement(&self) -> &DependencyRequirement {
        &self.requirement
    }

    /// Returns the command used for the check.
    #[must_use]
    pub fn command(&self) -> &[String] {
        &self.command
    }

    /// Returns how many probe attempts were needed before success.
    #[must_use]
    pub const fn attempts(&self) -> usize {
        self.attempts
    }
}

/// Report returned after dependency checks pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyReport {
    checks: Vec<DependencyCheck>,
}

impl DependencyReport {
    /// Creates a dependency report.
    #[must_use]
    pub fn new(checks: Vec<DependencyCheck>) -> Self {
        Self { checks }
    }

    /// Returns all completed checks.
    #[must_use]
    pub fn checks(&self) -> &[DependencyCheck] {
        &self.checks
    }
}

/// Executes a command probe.
pub trait CommandProbe {
    /// Runs a command with arguments.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be launched or exits
    /// unsuccessfully.
    fn probe(&self, program: &str, args: &[String]) -> Result<(), DependencyError>;
}

impl<T> CommandProbe for &T
where
    T: CommandProbe,
{
    fn probe(&self, program: &str, args: &[String]) -> Result<(), DependencyError> {
        (*self).probe(program, args)
    }
}

/// System command probe.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemCommandProbe;

impl CommandProbe for SystemCommandProbe {
    fn probe(&self, program: &str, args: &[String]) -> Result<(), DependencyError> {
        let output = process::output_strings(program, Path::new("."), args).map_err(|source| {
            DependencyError::CommandSpawn {
                program: program.to_owned(),
                source,
            }
        })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(DependencyError::CommandFailed {
                command: std::iter::once(program.to_owned())
                    .chain(args.iter().cloned())
                    .collect(),
                code: output.status.code(),
            })
        }
    }
}

/// Checks runtime dependencies.
#[derive(Clone, Debug)]
pub struct DependencyChecker<P> {
    probe: P,
}

impl<P> DependencyChecker<P>
where
    P: CommandProbe,
{
    /// Creates a dependency checker.
    #[must_use]
    pub const fn new(probe: P) -> Self {
        Self { probe }
    }

    /// Checks all requirements in order.
    ///
    /// # Errors
    ///
    /// Returns the first failed dependency check.
    pub fn check(&self, plan: &DependencyPlan) -> Result<DependencyReport, DependencyError> {
        let mut checks = Vec::with_capacity(plan.requirements().len());
        let registry = AdapterRegistry::default();

        for requirement in plan.requirements() {
            let command = DependencyCommand::for_requirement(requirement, &registry);
            let mut attempts = 0;
            let mut last_error = None;
            for attempt in 1..=DEPENDENCY_RETRY_ATTEMPTS {
                attempts = attempt;
                match self.probe.probe(command.program(), command.args()) {
                    Ok(()) => {
                        last_error = None;
                        break;
                    }
                    Err(error) => {
                        last_error = Some(error);
                    }
                }
            }
            if let Some(error) = last_error {
                return Err(error);
            }
            checks.push(DependencyCheck::new_with_attempts(
                requirement.clone(),
                command.as_strings(),
                attempts,
            ));
        }

        Ok(DependencyReport::new(checks))
    }
}

struct DependencyCommand {
    program: String,
    args: Vec<String>,
}

impl DependencyCommand {
    fn for_requirement(requirement: &DependencyRequirement, registry: &AdapterRegistry) -> Self {
        match requirement {
            DependencyRequirement::GitInstalled => Self::new(GIT_PROGRAM, [CLI_FLAG_VERSION]),
            DependencyRequirement::CargoInstalled => Self::new(CARGO_PROGRAM, [CLI_FLAG_VERSION]),
            DependencyRequirement::GitHubCliInstalled => {
                Self::new(GITHUB_CLI_PROGRAM, [CLI_FLAG_VERSION])
            }
            DependencyRequirement::GitHubCliLoggedIn => {
                Self::new(GITHUB_CLI_PROGRAM, [AUTH_ARG, STATUS_ARG])
            }
            DependencyRequirement::AgentCliInstalled(agent) => {
                Self::from_spec(&registry.installed_check(*agent))
            }
            DependencyRequirement::AgentCliLoggedIn(agent) => {
                Self::from_spec(&registry.auth_check(*agent))
            }
        }
    }

    fn new<const N: usize>(program: &str, args: [&str; N]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    fn from_spec(spec: &CommandSpec) -> Self {
        Self {
            program: spec.program().to_owned(),
            args: spec.args().to_vec(),
        }
    }

    fn program(&self) -> &str {
        &self.program
    }

    fn args(&self) -> &[String] {
        &self.args
    }

    fn as_strings(&self) -> Vec<String> {
        std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect()
    }
}

/// Dependency check error.
#[derive(Debug)]
pub enum DependencyError {
    /// A check command could not be launched.
    CommandSpawn {
        /// Program name.
        program: String,
        /// Original I/O error.
        source: io::Error,
    },
    /// A check command exited unsuccessfully.
    CommandFailed {
        /// Full command.
        command: Vec<String>,
        /// Process exit code.
        code: Option<i32>,
    },
}

impl fmt::Display for DependencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandSpawn { program, source } => {
                write!(f, "failed to launch dependency check `{program}`: {source}")
            }
            Self::CommandFailed { command, code } => {
                write!(
                    f,
                    "dependency check `{}` failed with exit code {code:?}",
                    command.join(" ")
                )
            }
        }
    }
}

impl std::error::Error for DependencyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CommandSpawn { source, .. } => Some(source),
            Self::CommandFailed { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, io};

    use crate::{
        Agent, CommandProbe, DependencyCheck, DependencyChecker, DependencyError, DependencyPlan,
        DependencyRequirement, SystemCommandProbe,
    };

    #[test]
    fn run_dependency_plan_requires_git_gh_and_agent_auth() {
        let agent = Agent::default();
        let judge_agent = agent.default_judge_for_worker();
        let plan = DependencyPlan::for_run(agent, judge_agent);

        assert_eq!(
            plan.requirements(),
            [
                DependencyRequirement::GitInstalled,
                DependencyRequirement::CargoInstalled,
                DependencyRequirement::GitHubCliInstalled,
                DependencyRequirement::GitHubCliLoggedIn,
                DependencyRequirement::AgentCliInstalled(agent),
                DependencyRequirement::AgentCliLoggedIn(agent),
                DependencyRequirement::AgentCliInstalled(judge_agent),
                DependencyRequirement::AgentCliLoggedIn(judge_agent)
            ]
        );
    }

    #[test]
    fn labels_dependency_requirements_for_user_output() {
        let agent = Agent::default();

        assert_eq!(DependencyRequirement::GitInstalled.label(), "git installed");
        assert_eq!(
            DependencyRequirement::CargoInstalled.label(),
            "cargo installed"
        );
        assert_eq!(
            DependencyRequirement::GitHubCliInstalled.label(),
            "gh installed"
        );
        assert_eq!(
            DependencyRequirement::GitHubCliLoggedIn.label(),
            "gh logged in"
        );
        assert_eq!(
            DependencyRequirement::AgentCliInstalled(agent).label(),
            "codex installed"
        );
        assert_eq!(
            DependencyRequirement::AgentCliLoggedIn(agent).label(),
            "codex logged in"
        );
    }

    #[derive(Debug, Default)]
    struct RecordingProbe {
        calls: RefCell<Vec<Vec<String>>>,
        fail_at: Option<usize>,
        always_fail_program: Option<String>,
    }

    impl RecordingProbe {
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl CommandProbe for RecordingProbe {
        fn probe(&self, program: &str, args: &[String]) -> Result<(), DependencyError> {
            let mut command = vec![program.to_owned()];
            command.extend(args.iter().cloned());
            self.calls.borrow_mut().push(command.clone());

            if self.fail_at == Some(self.calls.borrow().len())
                || self.always_fail_program.as_deref() == Some(program)
            {
                Err(DependencyError::CommandFailed {
                    command,
                    code: Some(1),
                })
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn checker_runs_dependency_commands_in_order() -> Result<(), DependencyError> {
        let probe = RecordingProbe::default();
        let checker = DependencyChecker::new(&probe);
        let agent = Agent::default();
        let report = checker.check(&DependencyPlan::for_run(
            agent,
            agent.default_judge_for_worker(),
        ))?;

        assert_eq!(report.checks().len(), 8);
        assert_eq!(
            probe.calls(),
            [
                vec!["git".to_owned(), "--version".to_owned()],
                vec!["cargo".to_owned(), "--version".to_owned()],
                vec!["gh".to_owned(), "--version".to_owned()],
                vec!["gh".to_owned(), "auth".to_owned(), "status".to_owned()],
                vec!["codex".to_owned(), "--version".to_owned()],
                vec!["codex".to_owned(), "login".to_owned(), "status".to_owned()],
                vec!["claude".to_owned(), "--version".to_owned()],
                vec!["claude".to_owned(), "auth".to_owned(), "status".to_owned()]
            ]
        );
        let first_check = report.checks().first();
        let expected_command = vec!["git".to_owned(), "--version".to_owned()];
        assert_eq!(
            first_check.map(DependencyCheck::command),
            Some(expected_command.as_slice())
        );
        assert_eq!(
            first_check.map(DependencyCheck::requirement),
            Some(&DependencyRequirement::GitInstalled)
        );

        Ok(())
    }

    #[test]
    fn checker_returns_first_failed_dependency() {
        let probe = RecordingProbe {
            calls: RefCell::new(Vec::new()),
            fail_at: None,
            always_fail_program: Some("cargo".to_owned()),
        };
        let checker = DependencyChecker::new(&probe);
        let agent = Agent::default();
        let result = checker.check(&DependencyPlan::for_run(
            agent,
            agent.default_judge_for_worker(),
        ));

        assert!(matches!(result, Err(DependencyError::CommandFailed { .. })));
        assert_eq!(probe.calls().len(), 4);
    }

    #[test]
    fn checker_retries_transient_dependency_failures() -> Result<(), DependencyError> {
        let probe = RecordingProbe {
            calls: RefCell::new(Vec::new()),
            fail_at: Some(2),
            always_fail_program: None,
        };
        let checker = DependencyChecker::new(&probe);
        let agent = Agent::default();
        let report = checker.check(&DependencyPlan::for_run(
            agent,
            agent.default_judge_for_worker(),
        ))?;

        let calls = probe.calls();
        assert_eq!(calls.len(), 9);
        assert_eq!(
            calls.get(1),
            Some(&vec!["cargo".to_owned(), "--version".to_owned()])
        );
        assert_eq!(
            calls.get(2),
            Some(&vec!["cargo".to_owned(), "--version".to_owned()])
        );
        assert_eq!(
            report.checks().get(1).map(DependencyCheck::attempts),
            Some(2)
        );
        assert_eq!(
            report.checks().first().map(DependencyCheck::attempts),
            Some(1)
        );
        Ok(())
    }

    #[test]
    fn displays_dependency_errors_and_sources() {
        let spawn = DependencyError::CommandSpawn {
            program: "git".to_owned(),
            source: io::Error::other("missing"),
        };
        let failed = DependencyError::CommandFailed {
            command: vec!["gh".to_owned(), "auth".to_owned(), "status".to_owned()],
            code: Some(1),
        };

        assert!(
            spawn
                .to_string()
                .contains("failed to launch dependency check")
        );
        assert!(
            failed
                .to_string()
                .contains("dependency check `gh auth status` failed")
        );
        assert!(std::error::Error::source(&spawn).is_some());
        assert!(std::error::Error::source(&failed).is_none());
    }

    #[test]
    fn system_command_probe_reports_process_results() -> Result<(), DependencyError> {
        let probe = SystemCommandProbe;
        probe.probe("sh", &["-c".to_owned(), "exit 0".to_owned()])?;

        let result = probe.probe("sh", &["-c".to_owned(), "exit 7".to_owned()]);

        assert!(matches!(
            result,
            Err(DependencyError::CommandFailed { code: Some(7), .. })
        ));
        Ok(())
    }

    #[test]
    fn system_command_probe_reports_spawn_errors() {
        let probe = SystemCommandProbe;
        let result = probe.probe("definitely-not-a-cyanos-command", &[]);

        assert!(matches!(result, Err(DependencyError::CommandSpawn { .. })));
    }
}
