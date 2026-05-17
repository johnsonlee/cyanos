//! Agent request model.

use super::ModelSelection;

use super::AgentExecutionMode;

/// Request passed from Cyanos to an underlying agent adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentRequest {
    model: ModelSelection,
    intent: Vec<String>,
    execution_mode: AgentExecutionMode,
}

impl AgentRequest {
    /// Creates an agent request.
    #[must_use]
    pub fn new(model: ModelSelection, intent: Vec<String>) -> Self {
        Self {
            model,
            intent,
            execution_mode: AgentExecutionMode::Worker,
        }
    }

    /// Creates an isolated judge request.
    #[must_use]
    pub fn judge_isolated(model: ModelSelection, intent: Vec<String>) -> Self {
        Self {
            model,
            intent,
            execution_mode: AgentExecutionMode::JudgeIsolated,
        }
    }

    /// Returns the model policy for this request.
    #[must_use]
    pub const fn model(&self) -> &ModelSelection {
        &self.model
    }

    /// Returns the user intent tokens for this request.
    #[must_use]
    pub fn intent(&self) -> &[String] {
        &self.intent
    }

    /// Returns the execution mode requested for the adapter command.
    #[must_use]
    pub const fn execution_mode(&self) -> AgentExecutionMode {
        self.execution_mode
    }

    /// Returns the complete prompt text for non-interactive CLIs.
    #[must_use]
    pub fn prompt(&self) -> String {
        self.intent.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentExecutionMode, AgentRequest, ModelSelection};

    #[test]
    fn exposes_request_fields() {
        let request = AgentRequest::new(
            ModelSelection::Explicit("model".to_owned()),
            vec!["one".to_owned(), "two".to_owned()],
        );

        assert_eq!(request.model().as_label(), "model");
        assert_eq!(request.intent(), ["one", "two"]);
        assert_eq!(request.execution_mode(), AgentExecutionMode::Worker);
        assert_eq!(request.prompt(), "one\n\ntwo");

        let judge =
            AgentRequest::judge_isolated(ModelSelection::BestSupported, vec!["review".to_owned()]);
        assert_eq!(
            judge.execution_mode().as_label(),
            "minimal-read-search-inspect"
        );
    }
}
