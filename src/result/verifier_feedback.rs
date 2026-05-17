//! Verifier feedback selected for one outer run.

use crate::json;

/// Verifier feedback selected for one outer run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifierFeedback {
    eval_path: String,
    summary: String,
    failure_class: String,
    findings: Vec<String>,
    benchmark_summary: String,
    next_action: String,
}

impl VerifierFeedback {
    /// Creates verifier feedback for a selected run.
    #[must_use]
    pub fn new(eval_path: String, summary: String, failure_class: String) -> Self {
        Self {
            eval_path,
            summary,
            failure_class,
            findings: Vec::new(),
            benchmark_summary: String::new(),
            next_action: String::new(),
        }
    }

    /// Adds verifier findings.
    #[must_use]
    pub fn with_findings(mut self, findings: Vec<String>) -> Self {
        self.findings = findings;
        self
    }

    /// Adds the benchmark summary.
    #[must_use]
    pub fn with_benchmark_summary(mut self, benchmark_summary: String) -> Self {
        self.benchmark_summary = benchmark_summary;
        self
    }

    /// Adds the next orchestrator action.
    #[must_use]
    pub fn with_next_action(mut self, next_action: String) -> Self {
        self.next_action = next_action;
        self
    }

    /// Returns the selected sample eval artifact path.
    #[must_use]
    pub fn eval_path(&self) -> &str {
        &self.eval_path
    }

    /// Returns the verifier summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Returns the failure class.
    #[must_use]
    pub fn failure_class(&self) -> &str {
        &self.failure_class
    }

    /// Returns verifier findings.
    #[must_use]
    pub fn findings(&self) -> &[String] {
        &self.findings
    }

    /// Returns benchmark summary text.
    #[must_use]
    pub fn benchmark_summary(&self) -> &str {
        &self.benchmark_summary
    }

    /// Returns the next action selected from verifier feedback.
    #[must_use]
    pub fn next_action(&self) -> &str {
        &self.next_action
    }

    pub(crate) fn to_json(&self, indent: &str) -> String {
        let findings = self
            .findings()
            .iter()
            .map(|finding| json::string(finding))
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "{{\n{indent}  \"eval_path\": {},\n{indent}  \"summary\": {},\n{indent}  \"failure_class\": {},\n{indent}  \"findings\": [{}],\n{indent}  \"benchmark_summary\": {},\n{indent}  \"next_action\": {}\n{indent}}}",
            json::string(self.eval_path()),
            json::string(self.summary()),
            json::string(self.failure_class()),
            findings,
            json::string(self.benchmark_summary()),
            json::string(self.next_action())
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::VerifierFeedback;

    #[test]
    fn exposes_verifier_feedback_fields() {
        let feedback = VerifierFeedback::new(
            "runs/1/samples/1/eval.json".to_owned(),
            "tests passed".to_owned(),
            "none".to_owned(),
        )
        .with_findings(vec!["coverage ok".to_owned()])
        .with_benchmark_summary("no regression".to_owned())
        .with_next_action("revise prompt".to_owned());

        assert_eq!(feedback.eval_path(), "runs/1/samples/1/eval.json");
        assert_eq!(feedback.summary(), "tests passed");
        assert_eq!(feedback.failure_class(), "none");
        assert_eq!(feedback.findings(), ["coverage ok"]);
        assert_eq!(feedback.benchmark_summary(), "no regression");
        assert_eq!(feedback.next_action(), "revise prompt");
    }
}
