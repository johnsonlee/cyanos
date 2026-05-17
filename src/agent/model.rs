//! Agent model selection.

use crate::terms::BEST_SUPPORTED_MODEL;

/// Model selection for the underlying agent.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ModelSelection {
    /// Use the best model supported by the selected agent.
    #[default]
    BestSupported,
    /// Use an explicit model name for the selected agent.
    Explicit(String),
}

impl ModelSelection {
    /// Returns a displayable model label.
    #[must_use]
    pub fn as_label(&self) -> &str {
        match self {
            Self::BestSupported => BEST_SUPPORTED_MODEL,
            Self::Explicit(model) => model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ModelSelection;

    #[test]
    fn exposes_model_label() {
        assert_eq!(ModelSelection::BestSupported.as_label(), "best-supported");
        assert_eq!(
            ModelSelection::Explicit("model".to_owned()).as_label(),
            "model"
        );
    }
}
