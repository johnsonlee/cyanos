//! Agent abstraction layer.

mod adapter;
mod command;
mod event;
mod execution_mode;
mod identity;
mod model;
mod observer;
mod output;
mod registry;
mod request;
mod runner;
mod runtime;
mod stream;
mod turn;

mod adapters;

pub use adapter::Adapter;
pub use command::CommandSpec;
pub use event::{AgentStreamEvent, AgentStreamToolCallEvent};
pub use execution_mode::AgentExecutionMode;
pub use identity::{Agent, AgentParseError};
pub use model::ModelSelection;
pub use observer::{AgentStreamObserver, NoopAgentStreamObserver};
pub use output::AgentOutput;
pub use registry::AdapterRegistry;
pub use request::AgentRequest;
pub use runner::{AgentCommandRunner, SystemAgentCommandRunner};
pub use runtime::{AgentRuntime, AgentRuntimeError, SystemAgentRuntime};
pub use stream::AgentStreamParser;
pub use turn::AgentTurn;
