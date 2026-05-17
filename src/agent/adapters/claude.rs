//! Claude CLI adapter.

use crate::terms::{
    AGENT_CLAUDE, AUTH_ARG, CLAUDE_BARE_FLAG, CLAUDE_DANGEROUS_FLAG,
    CLAUDE_DISABLE_SLASH_COMMANDS_FLAG, CLAUDE_PRINT_FLAG, CLAUDE_STRICT_MCP_CONFIG_FLAG,
    CLAUDE_TOOLS_FLAG, CLI_FLAG_VERSION, OUTPUT_FORMAT_FLAG, STATUS_ARG, STREAM_JSON_FORMAT,
};

use super::model_arg::append_model;
use crate::agent::{Adapter, Agent, AgentExecutionMode, AgentRequest, CommandSpec};

const EMPTY_MCP_CONFIG: &str = "{}";
const JUDGE_TOOLS: &str = "Read,Grep,Glob,LS";
const MCP_CONFIG_FLAG: &str = "--mcp-config";

/// Adapter for the Claude CLI.
#[derive(Clone, Copy, Debug)]
pub(in crate::agent) struct Claude {
    agent: Agent,
    program: &'static str,
}

impl Default for Claude {
    fn default() -> Self {
        Self {
            agent: Agent::Claude,
            program: AGENT_CLAUDE,
        }
    }
}

impl Adapter for Claude {
    fn agent(&self) -> Agent {
        self.agent
    }

    fn command(&self, request: &AgentRequest) -> CommandSpec {
        let mut args = vec![
            CLAUDE_PRINT_FLAG.to_owned(),
            OUTPUT_FORMAT_FLAG.to_owned(),
            STREAM_JSON_FORMAT.to_owned(),
        ];
        match request.execution_mode() {
            AgentExecutionMode::Worker => args.push(CLAUDE_DANGEROUS_FLAG.to_owned()),
            AgentExecutionMode::JudgeIsolated => args.extend([
                CLAUDE_BARE_FLAG.to_owned(),
                CLAUDE_DISABLE_SLASH_COMMANDS_FLAG.to_owned(),
                CLAUDE_STRICT_MCP_CONFIG_FLAG.to_owned(),
                MCP_CONFIG_FLAG.to_owned(),
                EMPTY_MCP_CONFIG.to_owned(),
                CLAUDE_TOOLS_FLAG.to_owned(),
                JUDGE_TOOLS.to_owned(),
            ]),
        }
        append_model(&mut args, request);
        CommandSpec::with_stdin(self.program, args, request.prompt())
    }

    fn installed_check(&self) -> CommandSpec {
        CommandSpec::new(self.program, vec![CLI_FLAG_VERSION.to_owned()])
    }

    fn auth_check(&self) -> CommandSpec {
        CommandSpec::new(
            self.program,
            vec![AUTH_ARG.to_owned(), STATUS_ARG.to_owned()],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Claude;
    use crate::agent::{Adapter, Agent, AgentRequest, ModelSelection};

    #[test]
    fn builds_claude_commands() {
        let adapter = Claude::default();
        let best = AgentRequest::new(ModelSelection::BestSupported, vec!["task".to_owned()]);
        let explicit = AgentRequest::new(
            ModelSelection::Explicit("model".to_owned()),
            vec!["task".to_owned()],
        );

        assert_eq!(adapter.agent(), Agent::Claude);
        assert_eq!(adapter.command(&best).program(), "claude");
        assert_eq!(
            adapter.command(&best).args(),
            [
                "-p",
                "--output-format",
                "stream-json",
                "--dangerously-skip-permissions"
            ]
        );
        assert_eq!(adapter.command(&best).stdin(), Some("task"));
        assert_eq!(
            adapter.command(&explicit).args(),
            [
                "-p",
                "--output-format",
                "stream-json",
                "--dangerously-skip-permissions",
                "--model",
                "model"
            ]
        );
        assert_eq!(adapter.command(&explicit).stdin(), Some("task"));
        let judge =
            AgentRequest::judge_isolated(ModelSelection::BestSupported, vec!["review".to_owned()]);
        assert_eq!(
            adapter.command(&judge).args(),
            [
                "-p",
                "--output-format",
                "stream-json",
                "--bare",
                "--disable-slash-commands",
                "--strict-mcp-config",
                "--mcp-config",
                "{}",
                "--tools",
                "Read,Grep,Glob,LS"
            ]
        );
        assert_eq!(adapter.command(&judge).stdin(), Some("review"));
        let long_prompt = "long prompt ".repeat(20_000);
        let long = AgentRequest::new(ModelSelection::BestSupported, vec![long_prompt.clone()]);
        let command = adapter.command(&long);
        assert!(!command.args().iter().any(|arg| arg.contains(&long_prompt)));
        assert_eq!(command.stdin(), Some(long_prompt.as_str()));
        assert_eq!(adapter.installed_check().args(), ["--version"]);
        assert_eq!(adapter.auth_check().args(), ["auth", "status"]);
    }
}
