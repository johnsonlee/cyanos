//! Agent execution mode.

const JUDGE_ISOLATION_LABEL: &str = "minimal-read-search-inspect";
const WORKER_EXECUTION_LABEL: &str = "worker-default";

/// Tool and configuration boundary for an agent execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentExecutionMode {
    /// Normal coding-worker mode.
    #[default]
    Worker,
    /// Read-only judge mode with project/user tools disabled where supported.
    JudgeIsolated,
}

impl AgentExecutionMode {
    /// Returns the evidence label for this execution mode.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Worker => WORKER_EXECUTION_LABEL,
            Self::JudgeIsolated => JUDGE_ISOLATION_LABEL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AgentExecutionMode;

    #[test]
    fn exposes_execution_mode_labels() {
        assert_eq!(AgentExecutionMode::Worker.as_label(), "worker-default");
        assert_eq!(
            AgentExecutionMode::JudgeIsolated.as_label(),
            "minimal-read-search-inspect"
        );
    }
}
