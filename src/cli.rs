//! Command-line application objects.

use std::fmt;

use crate::{
    agent::{AdapterRegistry, Agent, AgentRequest, CommandSpec, ModelSelection},
    terms::{
        CLI_COMMAND_INIT, CLI_COMMAND_RUN, CLI_FLAG_AGENT, CLI_FLAG_HELP, CLI_FLAG_MODEL,
        CLI_FLAG_PATH, CLI_FLAG_PREFIX, CLI_FLAG_PROJECT, CLI_FLAG_SAMPLES, CLI_FLAG_TASK,
        CLI_FLAG_VERSION, CLI_SHORT_HELP,
    },
    workspace::{IdentifierError, ProjectId, TaskId},
};

/// Cyanos command-line application.
#[derive(Clone, Debug, Default)]
pub struct CyanosCli {
    parser: CliParser,
}

impl CyanosCli {
    /// Product executable name.
    pub const PRODUCT_NAME: &'static str = "cyanos";

    /// Creates a CLI application with the provided parser.
    #[must_use]
    pub const fn new(parser: CliParser) -> Self {
        Self { parser }
    }

    /// Returns the command-line help text.
    #[must_use]
    pub fn help_text() -> String {
        format!(
            "{product} {version}\n\nUsage:\n  {product} init --project <repo-locator>\n  {product} run --project <repo-locator> --task <id> [--agent <agent>] [--model <model>] [--samples <n>]\n\nCommands:\n  init    Initialize a managed project from the current repository\n  run     Run the project task orchestrator\n\nOptions:\n  --project <repo-locator> Target repository locator\n  --task <id>             Task id to run\n  --agent <agent>         Agent CLI: claude, codex\n  --model <model>         Preferred model for the selected agent\n  --samples <n>           Number of isolated SSD samples to run\n  --help, -h              Print help\n  --version               Print version",
            product = Self::PRODUCT_NAME,
            version = env!("CARGO_PKG_VERSION"),
        )
    }

    /// Returns the command-line version text.
    #[must_use]
    pub fn version_text() -> String {
        format!("{} {}", Self::PRODUCT_NAME, env!("CARGO_PKG_VERSION"))
    }

    /// Parses and renders a CLI invocation.
    ///
    /// # Errors
    ///
    /// Returns an error when command-line parsing fails.
    pub fn render<I, S>(&self, args: I) -> Result<String, CliError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.parser
            .parse(args)
            .map(|invocation| invocation.render())
    }

    /// Parses CLI arguments into an invocation object.
    ///
    /// # Errors
    ///
    /// Returns an error when command-line parsing fails.
    pub fn parse<I, S>(&self, args: I) -> Result<Invocation, CliError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.parser.parse(args)
    }
}

/// Parser for Cyanos CLI arguments.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CliParser {
    default_agent: Agent,
}

impl CliParser {
    /// Creates a parser with the given default agent.
    #[must_use]
    pub const fn new(default_agent: Agent) -> Self {
        Self { default_agent }
    }

    /// Parses CLI arguments into an invocation.
    ///
    /// # Errors
    ///
    /// Returns an error when a command is missing, a required flag value is
    /// missing, a project id is invalid, or an unsupported command/flag/agent
    /// is provided.
    pub fn parse<I, S>(&self, args: I) -> Result<Invocation, CliError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut args = args.into_iter();
        let command = args.next().ok_or(CliError::MissingCommand)?;

        match command.as_ref() {
            CLI_FLAG_HELP | CLI_SHORT_HELP => Ok(Invocation::new(CliCommand::Help)),
            CLI_FLAG_VERSION => Ok(Invocation::new(CliCommand::Version)),
            CLI_COMMAND_INIT => Self::parse_init(args),
            CLI_COMMAND_RUN => self.parse_run(args),
            other => Err(CliError::UnknownCommand(other.to_owned())),
        }
    }

    fn parse_init<I, S>(args: I) -> Result<Invocation, CliError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut project_id = None;
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_ref() {
                CLI_FLAG_PROJECT => {
                    let value = args
                        .next()
                        .ok_or(CliError::MissingValue(CLI_FLAG_PROJECT))?;
                    project_id = Some(parse_project_id(value.as_ref())?);
                }
                CLI_FLAG_PATH => {
                    return Err(CliError::UnknownFlag(CLI_FLAG_PATH.to_owned()));
                }
                flag if flag.starts_with(CLI_FLAG_PREFIX) => {
                    return Err(CliError::UnknownFlag(flag.to_owned()));
                }
                value => return Err(CliError::UnexpectedArgument(value.to_owned())),
            }
        }

        Ok(Invocation::new(CliCommand::Init(InitCommand::new(
            project_id,
        ))))
    }

    fn parse_run<I, S>(&self, args: I) -> Result<Invocation, CliError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut project_id = None;
        let mut task_id = None;
        let mut agent = self.default_agent;
        let mut model = ModelSelection::default();
        let mut sample_count = RunCommand::DEFAULT_SAMPLE_COUNT;
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_ref() {
                CLI_FLAG_PROJECT => {
                    let value = args
                        .next()
                        .ok_or(CliError::MissingValue(CLI_FLAG_PROJECT))?;
                    project_id = Some(parse_project_id(value.as_ref())?);
                }
                CLI_FLAG_AGENT => {
                    let value = args.next().ok_or(CliError::MissingValue(CLI_FLAG_AGENT))?;
                    agent = value.as_ref().parse()?;
                }
                CLI_FLAG_TASK => {
                    let value = args.next().ok_or(CliError::MissingValue(CLI_FLAG_TASK))?;
                    task_id = Some(parse_task_id(value.as_ref())?);
                }
                CLI_FLAG_MODEL => {
                    let value = args.next().ok_or(CliError::MissingValue(CLI_FLAG_MODEL))?;
                    model = ModelSelection::Explicit(value.as_ref().to_owned());
                }
                CLI_FLAG_SAMPLES => {
                    let value = args
                        .next()
                        .ok_or(CliError::MissingValue(CLI_FLAG_SAMPLES))?;
                    sample_count = parse_sample_count(value.as_ref())?;
                }
                flag if flag.starts_with(CLI_FLAG_PREFIX) => {
                    return Err(CliError::UnknownFlag(flag.to_owned()));
                }
                value => return Err(CliError::UnexpectedArgument(value.to_owned())),
            }
        }

        let project_id = project_id.ok_or(CliError::MissingProjectId)?;
        let task_id = task_id.ok_or(CliError::MissingTaskId)?;
        Ok(Invocation::new(CliCommand::Run(RunCommand::new(
            project_id,
            task_id,
            agent,
            model,
            sample_count,
        ))))
    }
}

fn parse_project_id(value: &str) -> Result<ProjectId, CliError> {
    ProjectId::new(value).map_err(CliError::InvalidProjectId)
}

fn parse_task_id(value: &str) -> Result<TaskId, CliError> {
    TaskId::new(value.to_owned()).map_err(CliError::InvalidTaskId)
}

fn parse_sample_count(value: &str) -> Result<usize, CliError> {
    let sample_count = value
        .parse::<usize>()
        .map_err(|_error| CliError::InvalidSampleCount(value.to_owned()))?;
    if sample_count == 0 {
        return Err(CliError::InvalidSampleCount(value.to_owned()));
    }

    Ok(sample_count)
}

/// Parsed command-line invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invocation {
    command: CliCommand,
}

impl Invocation {
    /// Creates a parsed invocation.
    #[must_use]
    pub const fn new(command: CliCommand) -> Self {
        Self { command }
    }

    /// Returns the parsed command.
    #[must_use]
    pub const fn command(&self) -> &CliCommand {
        &self.command
    }

    /// Returns the run command when this invocation is `run`.
    #[must_use]
    pub const fn run_command(&self) -> Option<&RunCommand> {
        match &self.command {
            CliCommand::Run(command) => Some(command),
            CliCommand::Help | CliCommand::Version | CliCommand::Init(_) => None,
        }
    }

    /// Returns the adapter request represented by a `run` invocation.
    #[must_use]
    pub fn agent_request(&self) -> Option<AgentRequest> {
        self.run_command().map(|command| {
            AgentRequest::new(
                command.model().clone(),
                vec![
                    format!("project={}", command.project_id().as_str()),
                    format!("task={}", command.task_id().as_str()),
                ],
            )
        })
    }

    /// Builds the concrete command for the selected agent adapter.
    #[must_use]
    pub fn command_spec(&self) -> Option<CommandSpec> {
        self.run_command().map(|command| {
            let request = AgentRequest::new(
                command.model().clone(),
                vec![
                    format!("project={}", command.project_id().as_str()),
                    format!("task={}", command.task_id().as_str()),
                ],
            );
            AdapterRegistry::default().command(command.agent(), &request)
        })
    }

    /// Renders the invocation for the current scaffold executable.
    #[must_use]
    pub fn render(&self) -> String {
        let product_name = CyanosCli::PRODUCT_NAME;

        match &self.command {
            CliCommand::Help => CyanosCli::help_text(),
            CliCommand::Version => CyanosCli::version_text(),
            CliCommand::Init(command) => {
                let project = command
                    .project_id()
                    .map_or("interactive", ProjectId::as_str);
                format!("{product_name}: command=init project={project} path=current")
            }
            CliCommand::Run(command) => {
                let project = command.project_id().as_str();
                let task = command.task_id().as_str();
                let agent = command.agent().as_str();
                let model = command.model().as_label();
                let samples = command.sample_count();
                format!(
                    "{product_name}: command=run project={project} task={task} agent={agent} model={model} samples={samples}"
                )
            }
        }
    }
}

/// Supported top-level commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliCommand {
    /// Print command-line help.
    Help,
    /// Print the executable version.
    Version,
    /// Initialize a managed Cyanos project.
    Init(InitCommand),
    /// Run the task orchestrator for a managed project.
    Run(RunCommand),
}

/// `init` command options.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitCommand {
    project_id: Option<ProjectId>,
}

impl InitCommand {
    /// Creates an `init` command.
    #[must_use]
    pub const fn new(project_id: Option<ProjectId>) -> Self {
        Self { project_id }
    }

    /// Returns the project id, if provided non-interactively.
    #[must_use]
    pub const fn project_id(&self) -> Option<&ProjectId> {
        self.project_id.as_ref()
    }
}

/// `run` command options.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunCommand {
    project_id: ProjectId,
    task_id: TaskId,
    agent: Agent,
    model: ModelSelection,
    sample_count: usize,
}

impl RunCommand {
    /// Default number of samples for the alpha run path.
    pub const DEFAULT_SAMPLE_COUNT: usize = 1;

    /// Creates a `run` command.
    #[must_use]
    pub const fn new(
        project_id: ProjectId,
        task_id: TaskId,
        agent: Agent,
        model: ModelSelection,
        sample_count: usize,
    ) -> Self {
        Self {
            project_id,
            task_id,
            agent,
            model,
            sample_count,
        }
    }

    /// Returns the project id.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the task id.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the selected agent.
    #[must_use]
    pub const fn agent(&self) -> Agent {
        self.agent
    }

    /// Returns the selected model.
    #[must_use]
    pub const fn model(&self) -> &ModelSelection {
        &self.model
    }

    /// Returns the number of isolated SSD samples to run.
    #[must_use]
    pub const fn sample_count(&self) -> usize {
        self.sample_count
    }
}

/// Command-line parsing error.
#[derive(Debug, Eq, PartialEq)]
pub enum CliError {
    /// No top-level command was provided.
    MissingCommand,
    /// A required project id was missing.
    MissingProjectId,
    /// A required task id was missing.
    MissingTaskId,
    /// A flag that requires a value was missing that value.
    MissingValue(&'static str),
    /// An unsupported command was provided.
    UnknownCommand(String),
    /// An unsupported agent was requested.
    UnknownAgent(String),
    /// An unsupported flag was provided.
    UnknownFlag(String),
    /// A positional argument was not expected.
    UnexpectedArgument(String),
    /// The provided project id is not path-safe.
    InvalidProjectId(IdentifierError),
    /// The provided task id is not path-safe.
    InvalidTaskId(IdentifierError),
    /// The requested sample count is invalid.
    InvalidSampleCount(String),
}

impl From<crate::agent::AgentParseError> for CliError {
    fn from(error: crate::agent::AgentParseError) -> Self {
        Self::UnknownAgent(error.value().to_owned())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCommand => f.write_str("missing command"),
            Self::MissingProjectId => f.write_str("missing required --project"),
            Self::MissingTaskId => f.write_str("missing required --task"),
            Self::MissingValue(flag) => write!(f, "missing value for {flag}"),
            Self::UnknownCommand(command) => write!(f, "unknown command: {command}"),
            Self::UnknownAgent(agent) => write!(f, "unknown agent: {agent}"),
            Self::UnknownFlag(flag) => write!(f, "unknown flag: {flag}"),
            Self::UnexpectedArgument(value) => write!(f, "unexpected argument: {value}"),
            Self::InvalidProjectId(error) | Self::InvalidTaskId(error) => write!(f, "{error}"),
            Self::InvalidSampleCount(value) => {
                write!(
                    f,
                    "invalid --samples value: {value}; use a positive integer"
                )
            }
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidProjectId(error) | Self::InvalidTaskId(error) => Some(error),
            Self::MissingCommand
            | Self::MissingProjectId
            | Self::MissingTaskId
            | Self::MissingValue(_)
            | Self::UnknownCommand(_)
            | Self::UnknownAgent(_)
            | Self::UnknownFlag(_)
            | Self::UnexpectedArgument(_)
            | Self::InvalidSampleCount(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{CliCommand, CliError, CliParser, CyanosCli, Invocation};

    #[test]
    fn rejects_missing_command() {
        assert_eq!(
            CyanosCli::default().render(std::iter::empty::<&str>()),
            Err(CliError::MissingCommand)
        );
    }

    #[test]
    fn renders_help_and_version() -> Result<(), CliError> {
        let help = CyanosCli::default().render(["--help"])?;

        assert!(help.contains("Usage:"));
        assert!(help.contains("cyanos run --project <repo-locator>"));
        assert_eq!(
            CyanosCli::default().render(["--version"]),
            Ok(format!("cyanos {}", env!("CARGO_PKG_VERSION")))
        );

        Ok(())
    }

    #[test]
    fn renders_init_command_without_project_id_as_interactive() {
        assert_eq!(
            CyanosCli::default().render(["init"]),
            Ok("cyanos: command=init project=interactive path=current".to_owned())
        );
    }

    #[test]
    fn parses_init_project_locator() -> Result<(), CliError> {
        let invocation = CliParser::default().parse(["init", "--project", "owner/repo"])?;

        match invocation.command() {
            CliCommand::Init(command) => {
                assert_eq!(
                    command.project_id().map(crate::ProjectId::as_str),
                    Some("owner/repo")
                );
            }
            CliCommand::Help | CliCommand::Version | CliCommand::Run(_) => {
                return Err(CliError::UnknownCommand("run".to_owned()));
            }
        }

        Ok(())
    }

    #[test]
    fn parses_with_explicit_cli_parser_and_app() -> Result<(), CliError> {
        let app = CyanosCli::new(CliParser::new(crate::Agent::default()));
        let invocation = app.parse(["run", "--project", "demo", "--task", "123"])?;

        assert!(matches!(invocation.command(), CliCommand::Run(_)));
        assert_eq!(
            invocation
                .agent_request()
                .map(|request| request.intent().to_vec()),
            Some(vec!["project=demo".to_owned(), "task=123".to_owned()])
        );

        Ok(())
    }

    #[test]
    fn non_run_invocations_do_not_expose_run_details() -> Result<(), CliError> {
        let help = Invocation::new(CliCommand::Help);
        let init = CliParser::default().parse(["init", "--project", "demo"])?;

        assert!(help.run_command().is_none());
        assert!(help.agent_request().is_none());
        assert!(help.command_spec().is_none());
        assert!(init.run_command().is_none());

        Ok(())
    }

    #[test]
    fn renders_run_command_with_default_agent() {
        assert_eq!(
            CyanosCli::default().render(["run", "--project", "demo", "--task", "123"]),
            Ok(
                "cyanos: command=run project=demo task=123 agent=codex model=best-supported samples=1"
                    .to_owned()
            )
        );
    }

    #[test]
    fn parses_run_agent_and_model_options() -> Result<(), CliError> {
        let invocation = CliParser::default().parse([
            "run",
            "--project",
            "demo",
            "--task",
            "123",
            "--agent",
            "claude",
            "--model",
            "sonnet",
            "--samples",
            "3",
        ])?;
        let command = invocation
            .run_command()
            .ok_or_else(|| CliError::UnknownCommand("init".to_owned()))?;

        assert_eq!(command.project_id().as_str(), "demo");
        assert_eq!(command.task_id().as_str(), "123");
        assert_eq!(command.agent().as_str(), "claude");
        assert_eq!(command.model().as_label(), "sonnet");
        assert_eq!(command.sample_count(), 3);

        Ok(())
    }

    #[test]
    fn builds_command_spec_from_selected_adapter() -> Result<(), CliError> {
        let invocation = CliParser::default().parse([
            "run",
            "--project",
            "demo",
            "--task",
            "123",
            "--agent",
            "claude",
            "--model",
            "sonnet",
        ])?;
        let command = invocation
            .command_spec()
            .ok_or_else(|| CliError::UnknownCommand("init".to_owned()))?;

        assert_eq!(command.program(), "claude");
        assert_eq!(
            command.args(),
            [
                "-p",
                "--output-format",
                "stream-json",
                "--dangerously-skip-permissions",
                "--model",
                "sonnet"
            ]
        );
        assert_eq!(command.stdin(), Some("project=demo\n\ntask=123"));

        Ok(())
    }

    #[test]
    fn rejects_unknown_agent() {
        assert_eq!(
            CliParser::default().parse([
                "run",
                "--project",
                "demo",
                "--task",
                "123",
                "--agent",
                "unknown"
            ]),
            Err(CliError::UnknownAgent("unknown".to_owned()))
        );
    }

    #[test]
    fn rejects_missing_run_project_id() {
        assert_eq!(
            CliParser::default().parse(["run", "--task", "123"]),
            Err(CliError::MissingProjectId)
        );
    }

    #[test]
    fn rejects_missing_run_task_id() {
        assert_eq!(
            CliParser::default().parse(["run", "--project", "demo"]),
            Err(CliError::MissingTaskId)
        );
    }

    #[test]
    fn rejects_missing_flag_values() {
        assert_eq!(
            CliParser::default().parse(["init", "--project"]),
            Err(CliError::MissingValue("--project"))
        );
        assert_eq!(
            CliParser::default().parse(["run", "--project", "demo", "--agent"]),
            Err(CliError::MissingValue("--agent"))
        );
        assert_eq!(
            CliParser::default().parse(["run", "--project", "demo", "--task"]),
            Err(CliError::MissingValue("--task"))
        );
        assert_eq!(
            CliParser::default().parse(["run", "--project", "demo", "--task", "123", "--model"]),
            Err(CliError::MissingValue("--model"))
        );
        assert_eq!(
            CliParser::default().parse(["run", "--project", "demo", "--task", "123", "--samples"]),
            Err(CliError::MissingValue("--samples"))
        );
    }

    #[test]
    fn rejects_init_path_flag() {
        assert_eq!(
            CliParser::default().parse(["init", "--path", "."]),
            Err(CliError::UnknownFlag("--path".to_owned()))
        );
    }

    #[test]
    fn rejects_unknown_commands_flags_and_positionals() {
        assert_eq!(
            CliParser::default().parse(["status"]),
            Err(CliError::UnknownCommand("status".to_owned()))
        );
        assert_eq!(
            CliParser::default().parse(["init", "--bad"]),
            Err(CliError::UnknownFlag("--bad".to_owned()))
        );
        assert_eq!(
            CliParser::default().parse(["init", "--repo", "owner/repo"]),
            Err(CliError::UnknownFlag("--repo".to_owned()))
        );
        assert_eq!(
            CliParser::default().parse(["run", "--bad"]),
            Err(CliError::UnknownFlag("--bad".to_owned()))
        );
        assert_eq!(
            CliParser::default().parse(["init", "extra"]),
            Err(CliError::UnexpectedArgument("extra".to_owned()))
        );
        assert_eq!(
            CliParser::default().parse(["run", "extra"]),
            Err(CliError::UnexpectedArgument("extra".to_owned()))
        );
        assert_eq!(
            CliParser::default().parse([
                "run",
                "--project",
                "demo",
                "--task",
                "123",
                "--samples",
                "0"
            ]),
            Err(CliError::InvalidSampleCount("0".to_owned()))
        );
    }

    #[test]
    fn reports_invalid_project_id_with_source() {
        let result = CliParser::default().parse(["run", "--project", "../demo", "--task", "123"]);
        assert!(result.is_err());
        let Err(error) = result else {
            return;
        };

        assert_eq!(
            error.to_string(),
            "invalid project locator: use '<owner>/<repo>' or a full http(s) repository URL"
        );
        assert!(error.source().is_some());
    }

    #[test]
    fn displays_all_cli_error_variants() {
        assert_eq!(CliError::MissingCommand.to_string(), "missing command");
        assert_eq!(
            CliError::MissingProjectId.to_string(),
            "missing required --project"
        );
        assert_eq!(
            CliError::MissingTaskId.to_string(),
            "missing required --task"
        );
        assert_eq!(
            CliError::MissingValue("--flag").to_string(),
            "missing value for --flag"
        );
        assert_eq!(
            CliError::UnknownCommand("x".to_owned()).to_string(),
            "unknown command: x"
        );
        assert_eq!(
            CliError::UnknownFlag("--x".to_owned()).to_string(),
            "unknown flag: --x"
        );
        assert_eq!(
            CliError::UnexpectedArgument("x".to_owned()).to_string(),
            "unexpected argument: x"
        );
        assert_eq!(
            CliError::InvalidSampleCount("x".to_owned()).to_string(),
            "invalid --samples value: x; use a positive integer"
        );
    }
}
