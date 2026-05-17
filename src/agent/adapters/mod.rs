//! Concrete agent adapter package.

mod claude;
mod codex;
mod model_arg;

pub(in crate::agent) use claude::Claude;
pub(in crate::agent) use codex::Codex;
