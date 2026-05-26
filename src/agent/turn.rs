//! Agent turn model.

/// One turn sent to an underlying coding agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTurn {
    system_prompt: String,
    user_intent: String,
}

impl AgentTurn {
    /// Creates a turn for an agent.
    #[must_use]
    pub fn new(system_prompt: String, user_intent: String) -> Self {
        Self {
            system_prompt,
            user_intent,
        }
    }

    /// Returns the prompt used to guide the agent.
    #[must_use]
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    /// Returns the user's product intent.
    #[must_use]
    pub fn user_intent(&self) -> &str {
        &self.user_intent
    }
}

#[cfg(test)]
mod tests {
    use super::AgentTurn;

    #[test]
    fn exposes_turn_fields() {
        let turn = AgentTurn::new("system".to_owned(), "intent".to_owned());

        assert_eq!(turn.system_prompt(), "system");
        assert_eq!(turn.user_intent(), "intent");
    }
}
