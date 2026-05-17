//! Agent output model.

/// Output returned by an underlying agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentOutput {
    content: String,
}

impl AgentOutput {
    /// Creates agent output.
    #[must_use]
    pub fn new(content: String) -> Self {
        Self { content }
    }

    /// Returns the output content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

#[cfg(test)]
mod tests {
    use super::AgentOutput;

    #[test]
    fn exposes_output_content() {
        let output = AgentOutput::new("content".to_owned());

        assert_eq!(output.content(), "content");
    }
}
