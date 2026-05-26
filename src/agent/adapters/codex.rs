//! Codex CLI adapter.

use crate::terms::{
    AGENT_CODEX, CLI_FLAG_VERSION, CODEX_DANGEROUS_FLAG, CODEX_EPHEMERAL_FLAG,
    CODEX_IGNORE_RULES_FLAG, CODEX_IGNORE_USER_CONFIG_FLAG, CODEX_JSON_FLAG, CONFIG_ARG, EXEC_ARG,
    LOGIN_ARG, SANDBOX_FLAG, SANDBOX_READ_ONLY, STATUS_ARG,
};

use super::model_arg::append_model;
use crate::agent::{Adapter, Agent, AgentExecutionMode, AgentRequest, CommandSpec};

const EMPTY_MCP_SERVERS_CONFIG: &str = "mcp_servers={}";
const STDIN_PROMPT_ARG: &str = "-";

/// Adapter for the Codex CLI.
#[derive(Clone, Copy, Debug)]
pub(in crate::agent) struct Codex {
    agent: Agent,
    program: &'static str,
}

impl Default for Codex {
    fn default() -> Self {
        Self {
            agent: Agent::Codex,
            program: AGENT_CODEX,
        }
    }
}

impl Adapter for Codex {
    fn agent(&self) -> Agent {
        self.agent
    }

    fn command(&self, request: &AgentRequest) -> CommandSpec {
        let mut args = vec![EXEC_ARG.to_owned(), CODEX_JSON_FLAG.to_owned()];
        match request.execution_mode() {
            AgentExecutionMode::Worker => args.push(CODEX_DANGEROUS_FLAG.to_owned()),
            AgentExecutionMode::JudgeIsolated => args.extend([
                CODEX_IGNORE_USER_CONFIG_FLAG.to_owned(),
                CODEX_IGNORE_RULES_FLAG.to_owned(),
                CODEX_EPHEMERAL_FLAG.to_owned(),
                SANDBOX_FLAG.to_owned(),
                SANDBOX_READ_ONLY.to_owned(),
                CONFIG_ARG.to_owned(),
                EMPTY_MCP_SERVERS_CONFIG.to_owned(),
            ]),
        }
        append_model(&mut args, request);
        args.push(STDIN_PROMPT_ARG.to_owned());
        CommandSpec::with_stdin(self.program, args, request.prompt())
    }

    fn installed_check(&self) -> CommandSpec {
        CommandSpec::new(self.program, vec![CLI_FLAG_VERSION.to_owned()])
    }

    fn auth_check(&self) -> CommandSpec {
        CommandSpec::new(
            self.program,
            vec![LOGIN_ARG.to_owned(), STATUS_ARG.to_owned()],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Codex;
    use crate::agent::{Adapter, Agent, AgentRequest, ModelSelection};

    #[test]
    fn builds_codex_commands() {
        let adapter = Codex::default();
        let best = AgentRequest::new(ModelSelection::BestSupported, vec!["task".to_owned()]);
        let explicit = AgentRequest::new(
            ModelSelection::Explicit("model".to_owned()),
            vec!["task".to_owned()],
        );

        assert_eq!(adapter.agent(), Agent::Codex);
        assert_eq!(adapter.command(&best).program(), "codex");
        assert_eq!(
            adapter.command(&best).args(),
            [
                "exec",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "-"
            ]
        );
        assert_eq!(adapter.command(&best).stdin(), Some("task"));
        assert_eq!(
            adapter.command(&explicit).args(),
            [
                "exec",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "--model",
                "model",
                "-"
            ]
        );
        assert_eq!(adapter.command(&explicit).stdin(), Some("task"));
        let judge =
            AgentRequest::judge_isolated(ModelSelection::BestSupported, vec!["review".to_owned()]);
        assert_eq!(
            adapter.command(&judge).args(),
            [
                "exec",
                "--json",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--config",
                "mcp_servers={}",
                "-"
            ]
        );
        assert_eq!(adapter.command(&judge).stdin(), Some("review"));
        let long_prompt = "long prompt ".repeat(20_000);
        let long = AgentRequest::new(ModelSelection::BestSupported, vec![long_prompt.clone()]);
        let command = adapter.command(&long);
        assert!(!command.args().iter().any(|arg| arg.contains(&long_prompt)));
        assert_eq!(command.stdin(), Some(long_prompt.as_str()));
        assert_eq!(adapter.installed_check().args(), ["--version"]);
        assert_eq!(adapter.auth_check().args(), ["login", "status"]);
    }
}
