//! Adapter registry.

use super::{
    Adapter, Agent, AgentRequest, CommandSpec,
    adapters::{Claude, Codex},
};

/// Registry that hides concrete adapter selection from upper layers.
#[derive(Clone, Copy, Debug, Default)]
pub struct AdapterRegistry {
    claude: Claude,
    codex: Codex,
}

impl AdapterRegistry {
    /// Builds a command for the selected agent without exposing concrete
    /// adapters to callers.
    #[must_use]
    pub fn command(&self, agent: Agent, request: &AgentRequest) -> CommandSpec {
        match agent {
            Agent::Claude => self.claude.command(request),
            Agent::Codex => self.codex.command(request),
        }
    }

    /// Builds an installation check for the selected agent.
    #[must_use]
    pub fn installed_check(&self, agent: Agent) -> CommandSpec {
        match agent {
            Agent::Claude => self.claude.installed_check(),
            Agent::Codex => self.codex.installed_check(),
        }
    }

    /// Builds an authentication check for the selected agent.
    #[must_use]
    pub fn auth_check(&self, agent: Agent) -> CommandSpec {
        match agent {
            Agent::Claude => self.claude.auth_check(),
            Agent::Codex => self.codex.auth_check(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AdapterRegistry;
    use crate::agent::{Agent, AgentRequest, ModelSelection};

    #[test]
    fn dispatches_all_supported_agents() {
        let registry = AdapterRegistry::default();
        let request = AgentRequest::new(ModelSelection::BestSupported, vec!["task".to_owned()]);

        assert_eq!(
            registry.command(Agent::Claude, &request).program(),
            "claude"
        );
        assert_eq!(registry.command(Agent::Codex, &request).program(), "codex");
        assert_eq!(registry.installed_check(Agent::Claude).program(), "claude");
        assert_eq!(registry.installed_check(Agent::Codex).program(), "codex");
        assert_eq!(registry.auth_check(Agent::Claude).program(), "claude");
        assert_eq!(registry.auth_check(Agent::Codex).program(), "codex");
    }
}
