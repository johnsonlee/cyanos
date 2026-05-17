//! Agent stream observers.

use super::AgentStreamEvent;

/// Receives agent stream events while a concrete CLI is still running.
pub trait AgentStreamObserver {
    /// Handles one parsed stream event.
    fn observe(&mut self, event: &AgentStreamEvent);
}

/// Observer that intentionally ignores stream events.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoopAgentStreamObserver;

impl AgentStreamObserver for NoopAgentStreamObserver {
    fn observe(&mut self, _event: &AgentStreamEvent) {}
}

#[cfg(test)]
mod tests {
    use super::{AgentStreamObserver, NoopAgentStreamObserver};
    use crate::agent::{AgentStreamEvent, AgentStreamToolCallEvent};

    #[test]
    fn noop_observer_ignores_events() {
        let mut observer = NoopAgentStreamObserver;
        let event = AgentStreamEvent::ToolCall(AgentStreamToolCallEvent::new(
            "shell".to_owned(),
            "cargo test".to_owned(),
        ));

        observer.observe(&event);
    }
}
