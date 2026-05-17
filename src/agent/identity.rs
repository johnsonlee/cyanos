//! Supported agent identity.

use std::{fmt, str::FromStr};

use crate::terms::{AGENT_CLAUDE, AGENT_CODEX};

/// Supported underlying coding agent CLIs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Agent {
    /// Claude CLI.
    Claude,
    /// Codex CLI.
    #[default]
    Codex,
}

impl Agent {
    /// Returns the command-line name for the agent.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => AGENT_CLAUDE,
            Self::Codex => AGENT_CODEX,
        }
    }

    /// Returns the default independent judge agent for a worker agent.
    #[must_use]
    pub const fn default_judge_for_worker(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex => Self::Claude,
        }
    }
}

impl FromStr for Agent {
    type Err = AgentParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            AGENT_CLAUDE => Ok(Self::Claude),
            AGENT_CODEX => Ok(Self::Codex),
            other => Err(AgentParseError {
                value: other.to_owned(),
            }),
        }
    }
}

/// Error returned when an agent name cannot be parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentParseError {
    value: String,
}

impl AgentParseError {
    /// Returns the unsupported agent name.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for AgentParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = &self.value;
        write!(f, "unknown agent: {value}")
    }
}

impl std::error::Error for AgentParseError {}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::Agent;

    #[test]
    #[expect(
        clippy::expect_used,
        reason = "test fixture assertions require concrete values"
    )]
    fn parses_and_displays_agents() {
        assert_eq!(
            Agent::from_str("claude").expect("parse claude"),
            Agent::Claude
        );
        assert_eq!(Agent::from_str("codex").expect("parse codex"), Agent::Codex);
        assert_eq!(Agent::Claude.as_str(), "claude");
        assert_eq!(Agent::Codex.as_str(), "codex");
        assert_eq!(Agent::Claude.default_judge_for_worker(), Agent::Codex);
        assert_eq!(Agent::Codex.default_judge_for_worker(), Agent::Claude);

        let error = Agent::from_str("other").expect_err("unknown agent should fail");
        assert_eq!(error.value(), "other");
        assert_eq!(error.to_string(), "unknown agent: other");
    }
}
