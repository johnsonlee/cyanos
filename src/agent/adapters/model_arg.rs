//! Shared model argument adapter helper.

use crate::terms::CLI_FLAG_MODEL;

use super::super::{AgentRequest, ModelSelection};

pub(super) fn append_model(args: &mut Vec<String>, request: &AgentRequest) {
    if let ModelSelection::Explicit(model) = request.model() {
        args.push(CLI_FLAG_MODEL.to_owned());
        args.push(model.clone());
    }
}
