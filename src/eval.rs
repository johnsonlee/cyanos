//! Evaluation objects for agent attempt output.

use crate::agent::{AgentOutput, AgentTurn};
use crate::json;

const EVAL_FIELD_FINDINGS: &str = "\"findings\"";
const EVAL_FIELD_RECALL_EVIDENCE: &str = "\"recall_evidence\"";
const EVAL_FIELD_STRUCTURAL_EVIDENCE: &str = "\"structural_evidence\"";
const EVAL_FIELD_VERDICT: &str = "\"verdict\"";
const JSON_COLON: char = ':';
const JSON_COMMA: char = ',';
const JSON_ESCAPE: char = '\\';
const JSON_QUOTE: char = '"';
const JSON_UNICODE_ESCAPE: char = 'u';
const UNICODE_ESCAPE_DIGITS: usize = 4;
const UNICODE_ESCAPE_RADIX: u32 = 16;

/// Evaluation verdict for one agent attempt output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvaluationVerdict {
    /// Output satisfies the evaluator.
    Accepted,
    /// Output must be revised.
    Rejected,
}

impl EvaluationVerdict {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// One evaluation finding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationFinding {
    message: String,
}

impl EvaluationFinding {
    /// Creates an evaluation finding.
    #[must_use]
    pub fn new(message: String) -> Self {
        Self { message }
    }

    /// Returns the finding message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Result of evaluating agent attempt output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evaluation {
    verdict: EvaluationVerdict,
    structural_evidence: Vec<String>,
    recall_evidence: Vec<String>,
    findings: Vec<EvaluationFinding>,
}

impl Evaluation {
    /// Creates an accepted evaluation.
    #[must_use]
    pub fn accepted() -> Self {
        Self {
            verdict: EvaluationVerdict::Accepted,
            structural_evidence: Vec::new(),
            recall_evidence: Vec::new(),
            findings: Vec::new(),
        }
    }

    /// Creates an accepted evaluation with verifier evidence.
    #[must_use]
    pub fn accepted_with_findings(findings: Vec<EvaluationFinding>) -> Self {
        Self {
            verdict: EvaluationVerdict::Accepted,
            structural_evidence: Vec::new(),
            recall_evidence: Vec::new(),
            findings,
        }
    }

    /// Creates a rejected evaluation.
    #[must_use]
    pub fn rejected(findings: Vec<EvaluationFinding>) -> Self {
        Self {
            verdict: EvaluationVerdict::Rejected,
            structural_evidence: Vec::new(),
            recall_evidence: Vec::new(),
            findings,
        }
    }

    /// Adds verifier evidence grouped by score dimension.
    #[must_use]
    pub fn with_evidence(
        mut self,
        structural_evidence: Vec<String>,
        recall_evidence: Vec<String>,
    ) -> Self {
        self.structural_evidence = structural_evidence;
        self.recall_evidence = recall_evidence;
        self
    }

    /// Returns the verdict.
    #[must_use]
    pub const fn verdict(&self) -> EvaluationVerdict {
        self.verdict
    }

    /// Returns true when the output passed evaluation.
    #[must_use]
    pub const fn is_accepted(&self) -> bool {
        matches!(self.verdict, EvaluationVerdict::Accepted)
    }

    /// Returns structural verifier evidence.
    #[must_use]
    pub fn structural_evidence(&self) -> &[String] {
        &self.structural_evidence
    }

    /// Returns recall verifier evidence.
    #[must_use]
    pub fn recall_evidence(&self) -> &[String] {
        &self.recall_evidence
    }

    /// Returns evaluation findings.
    #[must_use]
    pub fn findings(&self) -> &[EvaluationFinding] {
        &self.findings
    }

    /// Renders the evaluation as a runtime JSON artifact.
    #[must_use]
    pub fn to_json(&self) -> String {
        let structural_evidence = json_array(self.structural_evidence());
        let recall_evidence = json_array(self.recall_evidence());
        let findings = self
            .findings()
            .iter()
            .map(|finding| json::string(finding.message()))
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "{{\n  \"verdict\": {},\n  \"structural_evidence\": [{}],\n  \"recall_evidence\": [{}],\n  \"findings\": [{}]\n}}\n",
            json::string(self.verdict().as_str()),
            structural_evidence,
            recall_evidence,
            findings
        )
    }
}

fn json_array(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json::string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Typed summary parsed from a sample `eval.json` artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationSnapshot {
    verdict: EvaluationVerdict,
    structural_evidence: Vec<String>,
    recall_evidence: Vec<String>,
    findings: Vec<String>,
}

impl EvaluationSnapshot {
    /// Parses a durable sample evaluation artifact.
    ///
    /// # Errors
    ///
    /// Returns an error when required fields are missing or malformed.
    pub fn parse(content: &str) -> Result<Self, EvaluationParseError> {
        let verdict = required_string_field(content, EVAL_FIELD_VERDICT).and_then(|value| {
            EvaluationVerdict::from_str(&value).ok_or(EvaluationParseError::InvalidVerdict(value))
        })?;

        Ok(Self {
            verdict,
            structural_evidence: required_string_array_field(
                content,
                EVAL_FIELD_STRUCTURAL_EVIDENCE,
            )?,
            recall_evidence: required_string_array_field(content, EVAL_FIELD_RECALL_EVIDENCE)?,
            findings: required_string_array_field(content, EVAL_FIELD_FINDINGS)?,
        })
    }

    /// Returns the parsed verdict.
    #[must_use]
    pub const fn verdict(&self) -> EvaluationVerdict {
        self.verdict
    }

    /// Returns structural evidence labels.
    #[must_use]
    pub fn structural_evidence(&self) -> &[String] {
        &self.structural_evidence
    }

    /// Returns recall evidence labels.
    #[must_use]
    pub fn recall_evidence(&self) -> &[String] {
        &self.recall_evidence
    }

    /// Returns parsed finding messages.
    #[must_use]
    pub fn findings(&self) -> &[String] {
        &self.findings
    }
}

fn required_string_field(
    content: &str,
    field: &'static str,
) -> Result<String, EvaluationParseError> {
    content
        .lines()
        .find_map(|line| json_string_field(line, field))
        .ok_or(EvaluationParseError::MissingField(field))
}

fn required_string_array_field(
    content: &str,
    field: &'static str,
) -> Result<Vec<String>, EvaluationParseError> {
    content
        .lines()
        .find_map(|line| json_string_array_field(line, field))
        .ok_or(EvaluationParseError::MissingField(field))
}

fn json_string_field(line: &str, field: &'static str) -> Option<String> {
    json_field_value(line, field).and_then(parse_json_string)
}

fn json_string_array_field(line: &str, field: &'static str) -> Option<Vec<String>> {
    json_field_value(line, field).and_then(parse_json_string_array)
}

fn json_field_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let field_start = line.find(field)?;
    let after_field = line.get(field_start + field.len()..)?;
    let value_start = after_field.find(JSON_COLON)?;
    after_field.get(value_start + 1..).map(str::trim)
}

fn parse_json_string(value: &str) -> Option<String> {
    let mut chars = value.trim().chars();
    parse_json_string_from_chars(&mut chars)
}

fn parse_json_string_from_chars<I>(chars: &mut I) -> Option<String>
where
    I: Iterator<Item = char>,
{
    if chars.next()? != JSON_QUOTE {
        return None;
    }
    let mut parsed = String::new();
    loop {
        let character = chars.next()?;
        match character {
            JSON_QUOTE => return Some(parsed),
            JSON_ESCAPE => parsed.push(parse_json_escape(chars)?),
            value => parsed.push(value),
        }
    }
}

fn parse_json_string_array(value: &str) -> Option<Vec<String>> {
    let trimmed = value.trim().trim_end_matches(JSON_COMMA).trim();
    let mut chars = trimmed.chars().peekable();
    skip_json_whitespace(&mut chars);
    if chars.next()? != '[' {
        return None;
    }
    let mut values = Vec::new();
    loop {
        skip_json_whitespace(&mut chars);
        match chars.peek().copied()? {
            ']' => {
                chars.next();
                return Some(values);
            }
            JSON_QUOTE => {
                values.push(parse_json_string_from_chars(&mut chars)?);
                skip_json_whitespace(&mut chars);
                match chars.peek().copied()? {
                    JSON_COMMA => {
                        chars.next();
                    }
                    ']' => {}
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
}

fn skip_json_whitespace<I>(chars: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = char>,
{
    while chars
        .peek()
        .is_some_and(|character| character.is_whitespace())
    {
        chars.next();
    }
}

fn parse_json_escape<I>(chars: &mut I) -> Option<char>
where
    I: Iterator<Item = char>,
{
    match chars.next()? {
        JSON_QUOTE => Some(JSON_QUOTE),
        JSON_ESCAPE => Some(JSON_ESCAPE),
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        JSON_UNICODE_ESCAPE => parse_unicode_escape(chars),
        _ => None,
    }
}

fn parse_unicode_escape<I>(chars: &mut I) -> Option<char>
where
    I: Iterator<Item = char>,
{
    let mut value = String::with_capacity(UNICODE_ESCAPE_DIGITS);
    for _ in 0..UNICODE_ESCAPE_DIGITS {
        value.push(chars.next()?);
    }
    u32::from_str_radix(&value, UNICODE_ESCAPE_RADIX)
        .ok()
        .and_then(char::from_u32)
}

/// Error returned when parsing sample `eval.json` as typed evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationParseError {
    /// A required evaluation field is missing or malformed.
    MissingField(&'static str),
    /// The evaluation verdict label is not supported.
    InvalidVerdict(String),
}

impl std::fmt::Display for EvaluationParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing evaluation field {field}"),
            Self::InvalidVerdict(value) => write!(f, "invalid evaluation verdict {value}"),
        }
    }
}

impl std::error::Error for EvaluationParseError {}

/// Object that evaluates one agent turn.
pub trait Evaluator {
    /// Evaluates one output from the agent.
    #[must_use]
    fn evaluate(&self, turn: &AgentTurn, output: &AgentOutput) -> Evaluation;
}

/// Minimal evaluator that rejects empty output.
#[derive(Clone, Copy, Debug)]
pub struct NonEmptyOutputEvaluator {
    empty_output_message: &'static str,
}

impl Default for NonEmptyOutputEvaluator {
    fn default() -> Self {
        Self {
            empty_output_message: "agent attempt returned empty output",
        }
    }
}

impl Evaluator for NonEmptyOutputEvaluator {
    fn evaluate(&self, _turn: &AgentTurn, output: &AgentOutput) -> Evaluation {
        if output.content().trim().is_empty() {
            Evaluation::rejected(vec![EvaluationFinding::new(
                self.empty_output_message.to_owned(),
            )])
        } else {
            Evaluation::accepted()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        AgentOutput, AgentTurn, Evaluation, EvaluationFinding, EvaluationParseError,
        EvaluationSnapshot, EvaluationVerdict, Evaluator, NonEmptyOutputEvaluator,
    };

    #[test]
    fn exposes_accepted_and_rejected_evaluation_fields() {
        let accepted = Evaluation::accepted();
        let accepted_with_findings =
            Evaluation::accepted_with_findings(vec![EvaluationFinding::new("passed".to_owned())]);
        let rejected = Evaluation::rejected(vec![EvaluationFinding::new("missing".to_owned())]);

        assert_eq!(accepted.verdict(), EvaluationVerdict::Accepted);
        assert!(accepted.findings().is_empty());
        assert!(accepted.structural_evidence().is_empty());
        assert!(accepted.recall_evidence().is_empty());
        assert_eq!(
            accepted_with_findings
                .findings()
                .first()
                .map(EvaluationFinding::message),
            Some("passed")
        );
        assert_eq!(rejected.verdict(), EvaluationVerdict::Rejected);
        assert_eq!(
            rejected.findings().first().map(EvaluationFinding::message),
            Some("missing")
        );
    }

    #[test]
    fn rejects_empty_output() {
        let evaluator = NonEmptyOutputEvaluator::default();
        let turn = AgentTurn::new("system".to_owned(), "intent".to_owned());
        let output = AgentOutput::new("   ".to_owned());
        let evaluation = evaluator.evaluate(&turn, &output);

        assert_eq!(evaluation.verdict(), EvaluationVerdict::Rejected);
        assert_eq!(
            evaluation
                .findings()
                .first()
                .map(EvaluationFinding::message),
            Some("agent attempt returned empty output")
        );
    }

    #[test]
    fn renders_evaluation_json() {
        let evaluation =
            Evaluation::rejected(vec![EvaluationFinding::new("missing output".to_owned())])
                .with_evidence(
                    vec!["cargo_check: failed".to_owned()],
                    vec!["coverage: not_measured".to_owned()],
                );

        assert_eq!(
            evaluation.to_json(),
            "{\n  \"verdict\": \"rejected\",\n  \"structural_evidence\": [\"cargo_check: failed\"],\n  \"recall_evidence\": [\"coverage: not_measured\"],\n  \"findings\": [\"missing output\"]\n}\n"
        );
    }

    #[test]
    fn parses_evaluation_snapshot_from_rendered_json() -> Result<(), Box<dyn std::error::Error>> {
        let evaluation = Evaluation::accepted_with_findings(vec![EvaluationFinding::new(
            "review \"ok\"".to_owned(),
        )])
        .with_evidence(
            vec!["cargo_check: passed".to_owned()],
            vec!["coverage: 9502".to_owned()],
        );

        let snapshot = EvaluationSnapshot::parse(&evaluation.to_json())?;

        assert_eq!(snapshot.verdict(), EvaluationVerdict::Accepted);
        assert_eq!(snapshot.structural_evidence(), ["cargo_check: passed"]);
        assert_eq!(snapshot.recall_evidence(), ["coverage: 9502"]);
        assert_eq!(snapshot.findings(), ["review \"ok\""]);
        assert!(matches!(
            EvaluationSnapshot::parse("{}"),
            Err(EvaluationParseError::MissingField(_))
        ));
        assert!(matches!(
            EvaluationSnapshot::parse(
                "{\n  \"verdict\": \"maybe\",\n  \"structural_evidence\": [],\n  \"recall_evidence\": [],\n  \"findings\": []\n}\n"
            ),
            Err(EvaluationParseError::InvalidVerdict(_))
        ));

        Ok(())
    }

    #[test]
    fn rejects_malformed_evaluation_snapshot_fields() {
        assert!(matches!(
            EvaluationSnapshot::parse(
                "{\n  \"verdict\": \"accepted\",\n  \"recall_evidence\": [],\n  \"findings\": []\n}\n"
            ),
            Err(EvaluationParseError::MissingField(_))
        ));
        assert!(matches!(
            EvaluationSnapshot::parse(
                "{\n  \"verdict\": \"accepted\",\n  \"structural_evidence\": \"bad\",\n  \"recall_evidence\": [],\n  \"findings\": []\n}\n"
            ),
            Err(EvaluationParseError::MissingField(_))
        ));
        assert!(matches!(
            EvaluationSnapshot::parse(
                "{\n  \"verdict\": \"accepted\",\n  \"structural_evidence\": [bad],\n  \"recall_evidence\": [],\n  \"findings\": []\n}\n"
            ),
            Err(EvaluationParseError::MissingField(_))
        ));
        assert!(matches!(
            EvaluationSnapshot::parse(
                "{\n  \"verdict\": \"accepted\",\n  \"structural_evidence\": [\"bad\\x\"],\n  \"recall_evidence\": [],\n  \"findings\": []\n}\n"
            ),
            Err(EvaluationParseError::MissingField(_))
        ));
        assert_eq!(
            EvaluationParseError::MissingField("\"verdict\"").to_string(),
            "missing evaluation field \"verdict\""
        );
        assert_eq!(
            EvaluationParseError::InvalidVerdict("maybe".to_owned()).to_string(),
            "invalid evaluation verdict maybe"
        );
    }

    #[test]
    fn parses_evaluation_snapshot_escaped_arrays() -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = EvaluationSnapshot::parse(
            "{\n  \"verdict\": \"rejected\",\n  \"structural_evidence\": [\"line\\n tab\\t return\\r quote\\\" slash\\\\ unicode\\u0008\", \"cargo: failed\"],\n  \"recall_evidence\": [\"coverage: 0\"],\n  \"findings\": [\"missing output\", \"fix it\"]\n}\n",
        )?;

        assert_eq!(snapshot.verdict(), EvaluationVerdict::Rejected);
        assert!(
            snapshot
                .structural_evidence()
                .first()
                .is_some_and(|value| value.contains("unicode\u{0008}"))
        );
        assert_eq!(
            snapshot.structural_evidence().get(1).map(String::as_str),
            Some("cargo: failed")
        );
        assert_eq!(
            snapshot.findings().get(1).map(String::as_str),
            Some("fix it")
        );

        Ok(())
    }
}
