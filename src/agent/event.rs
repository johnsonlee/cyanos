//! Agent stream events.

use std::fmt;

/// Tool-call event extracted from an agent JSON stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentStreamToolCallEvent {
    tool_name: String,
    summary: String,
}

impl AgentStreamToolCallEvent {
    /// Creates a tool-call stream event.
    #[must_use]
    pub fn new(tool_name: String, summary: String) -> Self {
        Self { tool_name, summary }
    }

    /// Returns the tool name.
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Returns the human-readable tool-call summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }
}

impl fmt::Display for AgentStreamToolCallEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}: {}]", self.tool_name(), self.summary())
    }
}

/// Event emitted while an agent command is running.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentStreamEvent {
    /// A tool call was observed in the agent stream.
    ToolCall(AgentStreamToolCallEvent),
}

impl fmt::Display for AgentStreamEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToolCall(event) => write!(f, "{event}"),
        }
    }
}
