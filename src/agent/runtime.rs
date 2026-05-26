//! System agent runtime.

use std::{fmt, path::PathBuf};

use super::{
    AdapterRegistry, Agent, AgentCommandRunner, AgentExecutionMode, AgentOutput, AgentRequest,
    AgentStreamObserver, AgentTurn, ModelSelection,
};

/// Error returned by an agent runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentRuntimeError {
    message: String,
}

impl AgentRuntimeError {
    /// Creates an agent runtime error.
    #[must_use]
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for AgentRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AgentRuntimeError {}

/// Runtime abstraction used by higher-level loops.
pub trait AgentRuntime {
    /// Executes one agent turn.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying agent runtime cannot complete the
    /// turn.
    fn execute(&mut self, turn: &AgentTurn) -> Result<AgentOutput, AgentRuntimeError>;

    /// Executes one agent turn while streaming parsed events to an observer.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying agent runtime cannot complete the
    /// turn.
    fn execute_with_observer(
        &mut self,
        turn: &AgentTurn,
        observer: &mut dyn AgentStreamObserver,
    ) -> Result<AgentOutput, AgentRuntimeError> {
        let _ = observer;
        self.execute(turn)
    }
}

/// Agent runtime that executes a selected CLI adapter in a worktree.
#[derive(Clone, Debug)]
pub struct SystemAgentRuntime<R> {
    agent: Agent,
    model: ModelSelection,
    execution_mode: AgentExecutionMode,
    cwd: PathBuf,
    runner: R,
    registry: AdapterRegistry,
}

impl<R> SystemAgentRuntime<R>
where
    R: AgentCommandRunner,
{
    /// Creates a system agent runtime.
    #[must_use]
    pub fn new(agent: Agent, model: ModelSelection, cwd: PathBuf, runner: R) -> Self {
        Self {
            agent,
            model,
            execution_mode: AgentExecutionMode::Worker,
            cwd,
            runner,
            registry: AdapterRegistry::default(),
        }
    }

    /// Creates a system agent runtime with an explicit execution mode.
    #[must_use]
    pub fn new_with_execution_mode(
        agent: Agent,
        model: ModelSelection,
        execution_mode: AgentExecutionMode,
        cwd: PathBuf,
        runner: R,
    ) -> Self {
        Self {
            agent,
            model,
            execution_mode,
            cwd,
            runner,
            registry: AdapterRegistry::default(),
        }
    }
}

impl<R> AgentRuntime for SystemAgentRuntime<R>
where
    R: AgentCommandRunner,
{
    fn execute(&mut self, turn: &AgentTurn) -> Result<AgentOutput, AgentRuntimeError> {
        let mut observer = super::NoopAgentStreamObserver;
        self.execute_with_observer(turn, &mut observer)
    }

    fn execute_with_observer(
        &mut self,
        turn: &AgentTurn,
        observer: &mut dyn AgentStreamObserver,
    ) -> Result<AgentOutput, AgentRuntimeError> {
        let intent = vec![
            format!("System prompt:\n{}", turn.system_prompt()),
            format!("User intent:\n{}", turn.user_intent()),
        ];
        let request = match self.execution_mode {
            AgentExecutionMode::Worker => AgentRequest::new(self.model.clone(), intent),
            AgentExecutionMode::JudgeIsolated => {
                AgentRequest::judge_isolated(self.model.clone(), intent)
            }
        };
        let command = self.registry.command(self.agent, &request);
        self.runner
            .run_with_observer(&self.cwd, &command, observer)
            .map(AgentOutput::new)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        path::{Path, PathBuf},
    };

    use super::{AgentRuntime, AgentRuntimeError, SystemAgentRuntime};
    use crate::agent::{
        Agent, AgentCommandRunner, AgentExecutionMode, AgentTurn, CommandSpec, ModelSelection,
    };

    #[derive(Debug, Default)]
    struct RecordingAgentRunner {
        calls: RefCell<Vec<(PathBuf, CommandSpec)>>,
        fail: bool,
    }

    impl RecordingAgentRunner {
        fn calls(&self) -> Vec<(PathBuf, CommandSpec)> {
            self.calls.borrow().clone()
        }
    }

    impl AgentCommandRunner for RecordingAgentRunner {
        fn run(&self, cwd: &Path, command: &CommandSpec) -> Result<String, AgentRuntimeError> {
            self.calls
                .borrow_mut()
                .push((cwd.to_path_buf(), command.clone()));

            if self.fail {
                Err(AgentRuntimeError::new("agent failed".to_owned()))
            } else {
                Ok("agent output".to_owned())
            }
        }
    }

    #[test]
    fn system_agent_runtime_executes_selected_adapter() -> Result<(), AgentRuntimeError> {
        let runner = RecordingAgentRunner::default();
        let mut runtime = SystemAgentRuntime::new(
            Agent::Codex,
            ModelSelection::Explicit("gpt-5.5".to_owned()),
            PathBuf::from("/tmp/worktree"),
            &runner,
        );
        let turn = AgentTurn::new("inner prompt".to_owned(), "task details".to_owned());

        let output = runtime.execute(&turn)?;

        assert_eq!(output.content(), "agent output");
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let first = calls.first();
        let expected_args = vec![
            "exec".to_owned(),
            "--json".to_owned(),
            "--dangerously-bypass-approvals-and-sandbox".to_owned(),
            "--model".to_owned(),
            "gpt-5.5".to_owned(),
            "-".to_owned(),
        ];
        assert_eq!(
            first.map(|call| call.0.as_path()),
            Some(Path::new("/tmp/worktree"))
        );
        assert_eq!(first.map(|call| call.1.program()), Some("codex"));
        assert_eq!(
            first.map(|call| call.1.args()),
            Some(expected_args.as_slice())
        );
        assert_eq!(
            first.and_then(|call| call.1.stdin()),
            Some("System prompt:\ninner prompt\n\nUser intent:\ntask details")
        );

        Ok(())
    }

    #[test]
    fn system_agent_runtime_executes_judge_with_isolated_mode() -> Result<(), AgentRuntimeError> {
        let runner = RecordingAgentRunner::default();
        let mut runtime = SystemAgentRuntime::new_with_execution_mode(
            Agent::Codex,
            ModelSelection::BestSupported,
            AgentExecutionMode::JudgeIsolated,
            PathBuf::from("/tmp/judge"),
            &runner,
        );
        let turn = AgentTurn::new("judge prompt".to_owned(), "candidate diff".to_owned());

        runtime.execute(&turn)?;

        let calls = runner.calls();
        let Some(first) = calls.first() else {
            return Err(AgentRuntimeError::new("missing command call".to_owned()));
        };
        assert_eq!(first.1.program(), "codex");
        assert!(first.1.args().contains(&"--ignore-user-config".to_owned()));
        assert!(first.1.args().contains(&"--ignore-rules".to_owned()));
        assert!(
            !first
                .1
                .args()
                .contains(&"--dangerously-bypass-approvals-and-sandbox".to_owned())
        );
        assert_eq!(
            first.1.stdin(),
            Some("System prompt:\njudge prompt\n\nUser intent:\ncandidate diff")
        );
        Ok(())
    }

    #[test]
    fn system_agent_runtime_propagates_runner_errors() {
        let runner = RecordingAgentRunner {
            calls: RefCell::new(Vec::new()),
            fail: true,
        };
        let mut runtime = SystemAgentRuntime::new(
            Agent::default(),
            ModelSelection::BestSupported,
            PathBuf::from("/tmp/worktree"),
            runner,
        );
        let result = runtime.execute(&AgentTurn::new("prompt".to_owned(), "task".to_owned()));

        assert!(matches!(result, Err(AgentRuntimeError { .. })));
    }
}
