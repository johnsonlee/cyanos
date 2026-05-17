//! Agent adapter trait.

use super::{Agent, AgentRequest, CommandSpec};

/// Adapter implemented by each supported coding agent CLI.
pub trait Adapter {
    /// Returns the agent represented by this adapter.
    #[must_use]
    fn agent(&self) -> Agent;

    /// Builds the concrete CLI command for a request.
    #[must_use]
    fn command(&self, request: &AgentRequest) -> CommandSpec;

    /// Builds a non-interactive CLI command that verifies the executable is
    /// available.
    #[must_use]
    fn installed_check(&self) -> CommandSpec;

    /// Builds a non-interactive CLI command that verifies authentication.
    #[must_use]
    fn auth_check(&self) -> CommandSpec;
}
