//! GitHub task, PR, and review-comment lifecycle helpers.

use std::collections::HashSet;

use crate::{
    AgentRuntimeError, AppError, JUDGE_PERCENT_TO_BASIS_POINTS, JUDGE_RUBRIC_RULE, JudgeFinding,
    judge_recall_from_findings,
};

const CYANOS_COMMENT_MARKER_PREFIX: &str = "cyanos:";
const CYANOS_RESOLUTION_COMMIT_FIELD: &str = "commit=";
const CYANOS_RESOLUTION_EVAL_FIELD: &str = "eval=";
const CYANOS_RESOLUTION_FIXED_MARKER: &str = "cyanos:resolution=fixed";
const CYANOS_REVIEW_THREAD_FIXED_MARKER: &str = "cyanos:review-thread=fixed";
const CYANOS_REVIEW_THREAD_FIELD: &str = "thread=";
const CYANOS_REVIEW_THREAD_EVIDENCE_FIELD: &str = "evidence=";
const REVIEW_THREAD_EVIDENCE_JUDGE: &str = "judge";
const REVIEW_THREAD_EVIDENCE_VERIFIER: &str = "verifier";
const REVIEW_COMMENT_MARKER_CYANOS: &str = "cyanos";
const REVIEW_COMMENT_MARKER_EXTERNAL: &str = "external";
const STATE_VALUE_MAX_CHARS: usize = 120;

/// Normalized PR review comment record from the GitHub review-thread API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewCommentRecord {
    /// GitHub review thread GraphQL id.
    pub thread_id: String,
    /// Numeric GitHub pull-request review comment id used for thread replies.
    pub comment_id: String,
    /// Whether the review thread is already resolved.
    pub is_resolved: bool,
    /// Whether the review thread is outdated.
    pub is_outdated: bool,
    /// Repository path associated with the comment.
    pub path: String,
    /// Review comment author.
    pub author: String,
    /// Review comment URL.
    pub url: String,
    /// Review comment body.
    pub body: String,
}

/// Current PR evidence used to decide whether a review thread still applies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReviewThreadEvidence<'a> {
    current_diff: &'a str,
    managed_comment_author: Option<&'a str>,
    fixed_thread_ids: &'a [String],
}

impl<'a> ReviewThreadEvidence<'a> {
    /// Creates current review-thread evidence from the PR diff.
    #[must_use]
    pub const fn new(current_diff: &'a str) -> Self {
        Self {
            current_diff,
            managed_comment_author: None,
            fixed_thread_ids: &[],
        }
    }

    /// Adds the GitHub actor that Cyanos uses to write managed review comments.
    #[must_use]
    pub const fn with_managed_comment_author(self, author: &'a str) -> Self {
        Self {
            current_diff: self.current_diff,
            managed_comment_author: Some(author),
            fixed_thread_ids: self.fixed_thread_ids,
        }
    }

    /// Adds verifier or judge evidence that specific review threads are fixed.
    #[must_use]
    pub const fn with_fixed_thread_ids(self, fixed_thread_ids: &'a [String]) -> Self {
        Self {
            current_diff: self.current_diff,
            managed_comment_author: self.managed_comment_author,
            fixed_thread_ids,
        }
    }
}

/// Parses review-thread TSV emitted by the `gh api graphql` query.
///
/// # Errors
///
/// Returns an error when a row has an unexpected field count or invalid
/// boolean fields.
pub fn review_comment_records_from_tsv(
    output: &[u8],
) -> Result<Vec<ReviewCommentRecord>, AppError> {
    let mut records = Vec::new();
    for (line_index, line) in String::from_utf8_lossy(output)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let fields = line.split('\t').collect::<Vec<_>>();
        let [
            thread_id,
            is_resolved,
            is_outdated,
            path,
            comment_id,
            author,
            url,
            body,
        ] = fields.as_slice()
        else {
            return Err(AgentRuntimeError::new(format!(
                "failed to parse PR review comment record at line {}; expected 8 TSV fields but found {}; next action: rerun with a current gh CLI",
                line_index + 1,
                fields.len()
            ))
            .into());
        };
        records.push(ReviewCommentRecord {
            thread_id: (*thread_id).to_owned(),
            comment_id: (*comment_id).to_owned(),
            is_resolved: parse_review_thread_bool(is_resolved, line_index + 1, "isResolved")?,
            is_outdated: parse_review_thread_bool(is_outdated, line_index + 1, "isOutdated")?,
            path: (*path).to_owned(),
            author: (*author).to_owned(),
            url: (*url).to_owned(),
            body: (*body).to_owned(),
        });
    }

    Ok(records)
}

/// Parses an unresolved review-thread count.
///
/// # Errors
///
/// Returns an error when the output is not an integer.
pub fn parse_unresolved_review_thread_count(output: &[u8]) -> Result<usize, AppError> {
    String::from_utf8_lossy(output)
        .trim()
        .parse::<usize>()
        .map_err(|source| {
            AgentRuntimeError::new(format!(
                "failed to parse unresolved review thread count: {source}; next action: rerun with a current gh CLI"
            ))
            .into()
        })
}

/// Returns unresolved outdated thread ids that Cyanos can resolve.
#[must_use]
pub fn unresolved_outdated_thread_ids(records: &[ReviewCommentRecord]) -> Vec<String> {
    let mut thread_ids = Vec::new();
    for record in records {
        if !record.is_resolved
            && record.is_outdated
            && !thread_ids
                .iter()
                .any(|thread_id: &String| thread_id == &record.thread_id)
        {
            thread_ids.push(record.thread_id.clone());
        }
    }
    thread_ids
}

/// Returns unresolved current thread ids whose discussion records a fixed finding.
#[must_use]
pub fn unresolved_fixed_thread_ids(records: &[ReviewCommentRecord]) -> Vec<String> {
    unresolved_fixed_thread_ids_with_evidence(records, ReviewThreadEvidence::default())
}

/// Returns unresolved current thread ids whose findings are fixed by evidence.
#[must_use]
pub fn unresolved_fixed_thread_ids_with_evidence(
    records: &[ReviewCommentRecord],
    evidence: ReviewThreadEvidence<'_>,
) -> Vec<String> {
    let mut thread_ids = Vec::new();
    for record in records {
        if record.is_resolved || record.is_outdated || !review_thread_is_fixed(record, evidence) {
            continue;
        }
        if !thread_ids
            .iter()
            .any(|thread_id: &String| thread_id == &record.thread_id)
        {
            thread_ids.push(record.thread_id.clone());
        }
    }
    thread_ids
}

/// Returns review-thread ids marked fixed by verifier or judge evidence.
#[must_use]
pub fn fixed_review_thread_ids_from_verifier_evidence(evidence: &[String]) -> Vec<String> {
    let mut thread_ids = Vec::new();
    for item in evidence {
        let Some(thread_id) = review_thread_id_from_verifier_evidence(item) else {
            continue;
        };
        if !thread_ids
            .iter()
            .any(|existing: &String| existing == &thread_id)
        {
            thread_ids.push(thread_id);
        }
    }
    thread_ids
}

/// Converts active review comments into normalized verifier feedback.
#[must_use]
pub fn review_feedback_from_comment_records(records: &[ReviewCommentRecord]) -> Option<String> {
    review_feedback_from_comment_records_with_evidence(records, ReviewThreadEvidence::default())
}

/// Converts still-valid review comments into normalized verifier feedback.
#[must_use]
pub fn review_feedback_from_comment_records_with_evidence(
    records: &[ReviewCommentRecord],
    evidence: ReviewThreadEvidence<'_>,
) -> Option<String> {
    let fixed_thread_ids = unresolved_fixed_thread_ids_with_evidence(records, evidence)
        .into_iter()
        .collect::<HashSet<_>>();
    let findings = records
        .iter()
        .filter(|record| {
            !record.is_resolved
                && !record.is_outdated
                && !fixed_thread_ids.contains(&record.thread_id)
        })
        .map(review_comment_to_finding)
        .collect::<Vec<_>>();
    if findings.is_empty() {
        return None;
    }

    let score_basis_points = judge_recall_from_findings(&findings);
    let findings_evidence = findings
        .iter()
        .map(JudgeFinding::to_evidence)
        .collect::<Vec<_>>()
        .join("; ");
    let mut feedback = vec![format!(
        "PR readiness failed; source=github_review code_review_recall={}.{:02}% rule={} findings=[{}]",
        score_basis_points / 100,
        score_basis_points % JUDGE_PERCENT_TO_BASIS_POINTS,
        JUDGE_RUBRIC_RULE,
        findings_evidence
    )];
    feedback.extend(findings.iter().map(JudgeFinding::to_message));
    Some(feedback.join("\n"))
}

fn review_thread_is_fixed(
    record: &ReviewCommentRecord,
    evidence: ReviewThreadEvidence<'_>,
) -> bool {
    comment_has_cyanos_fixed_resolution(record, evidence.managed_comment_author)
        || review_thread_fixed_by_verifier(record, evidence)
        || review_path_removed_from_current_diff(record, evidence)
}

/// Returns whether a thread already contains a Cyanos-owned fixed-resolution marker.
#[must_use]
pub fn review_thread_has_managed_fixed_resolution(
    records: &[ReviewCommentRecord],
    thread_id: &str,
    managed_comment_author: &str,
) -> bool {
    records.iter().any(|record| {
        record.thread_id == thread_id
            && comment_has_cyanos_fixed_resolution(record, Some(managed_comment_author))
    })
}

fn comment_has_cyanos_fixed_resolution(
    record: &ReviewCommentRecord,
    managed_comment_author: Option<&str>,
) -> bool {
    let Some(managed_comment_author) = managed_comment_author else {
        return false;
    };
    if !record
        .author
        .trim()
        .eq_ignore_ascii_case(managed_comment_author)
    {
        return false;
    }
    let lowered = record.body.to_ascii_lowercase();
    lowered.contains(CYANOS_RESOLUTION_FIXED_MARKER)
        && lowered.contains(CYANOS_RESOLUTION_COMMIT_FIELD)
        && lowered.contains(CYANOS_RESOLUTION_EVAL_FIELD)
}

fn review_thread_fixed_by_verifier(
    record: &ReviewCommentRecord,
    evidence: ReviewThreadEvidence<'_>,
) -> bool {
    evidence
        .fixed_thread_ids
        .iter()
        .any(|thread_id| thread_id == &record.thread_id)
}

fn review_thread_id_from_verifier_evidence(item: &str) -> Option<String> {
    let mut thread_id = None;
    let mut has_marker = false;
    let mut has_trusted_evidence = false;
    for token in item.split_whitespace() {
        if token.eq_ignore_ascii_case(CYANOS_REVIEW_THREAD_FIXED_MARKER) {
            has_marker = true;
            continue;
        }
        if let Some(value) = token.strip_prefix(CYANOS_REVIEW_THREAD_FIELD) {
            if !value.trim().is_empty() {
                thread_id = Some(value.trim().to_owned());
            }
            continue;
        }
        if let Some(value) = token.strip_prefix(CYANOS_REVIEW_THREAD_EVIDENCE_FIELD) {
            has_trusted_evidence = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                REVIEW_THREAD_EVIDENCE_JUDGE | REVIEW_THREAD_EVIDENCE_VERIFIER
            );
        }
    }

    if has_marker && has_trusted_evidence {
        thread_id
    } else {
        None
    }
}

fn review_path_removed_from_current_diff(
    record: &ReviewCommentRecord,
    evidence: ReviewThreadEvidence<'_>,
) -> bool {
    if record.path.trim().is_empty() || evidence.current_diff.trim().is_empty() {
        return false;
    }
    let path = record.path.trim();
    let diff_header = format!("diff --git a/{path} b/{path}");
    let new_file = format!("+++ b/{path}");
    let old_file = format!("--- a/{path}");
    !evidence.current_diff.contains(&diff_header)
        && !evidence.current_diff.contains(&new_file)
        && !evidence.current_diff.contains(&old_file)
}

/// Converts one review comment into a normalized judge finding.
#[must_use]
pub fn review_comment_to_finding(record: &ReviewCommentRecord) -> JudgeFinding {
    let marker = review_comment_marker(&record.body);
    let evidence = review_comment_evidence(record, marker);
    let body = normalize_review_text(&record.body);
    if body.is_empty() {
        return JudgeFinding::invalid_review_comment(
            "invalid empty PR review comment",
            "remove stale empty review comment or replace it with category severity concern fix evidence",
            &evidence,
        );
    }

    if let Some(finding) = structured_review_comment_finding(
        &normalize_structured_review_text(&record.body),
        &evidence,
    ) {
        return finding;
    }

    JudgeFinding::invalid_review_comment(&body, &review_comment_fix(&body), &evidence)
}

fn parse_review_thread_bool(
    value: &str,
    line_number: usize,
    field: &str,
) -> Result<bool, AppError> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(AgentRuntimeError::new(format!(
            "failed to parse PR review comment record at line {line_number}; {field}={value} is not a boolean"
        ))
        .into()),
    }
}

fn structured_review_comment_finding(body: &str, evidence: &str) -> Option<JudgeFinding> {
    let structured_fields = ["category=", "severity=", "concern=", "fix=", "evidence="];
    let Some(category_start) = body.find("category=") else {
        return structured_fields.iter().any(|field| body.contains(field)).then(|| {
            JudgeFinding::invalid_review_comment(
                "invalid structured PR review comment missing category severity concern fix or evidence",
                "rewrite review comment with comma-separated category severity concern fix and evidence fields",
                evidence,
            )
        });
    };
    let segment = &body[category_start..];
    if let Some(finding) = JudgeFinding::from_segment(segment) {
        return Some(finding.with_evidence_suffix(evidence));
    }

    Some(JudgeFinding::invalid_review_comment(
        "invalid structured PR review comment missing category severity concern fix or evidence",
        "rewrite review comment with comma-separated category severity concern fix and evidence fields",
        evidence,
    ))
}

fn review_comment_marker(body: &str) -> &'static str {
    if body.contains(CYANOS_COMMENT_MARKER_PREFIX) || body.contains("CYANOS_JUDGE") {
        REVIEW_COMMENT_MARKER_CYANOS
    } else {
        REVIEW_COMMENT_MARKER_EXTERNAL
    }
}

fn review_comment_evidence(record: &ReviewCommentRecord, marker: &str) -> String {
    format!(
        "github_review url={} author={} marker={} path={}",
        normalize_review_text(&record.url),
        normalize_review_text(&record.author),
        marker,
        normalize_review_text(&record.path)
    )
}

fn review_comment_fix(body: &str) -> String {
    let lower = body.to_ascii_lowercase();
    for needle in ["please ", "fix ", "add ", "remove ", "change ", "make "] {
        if let Some(start) = lower.find(needle) {
            return truncate_review_text(&body[start..], STATE_VALUE_MAX_CHARS);
        }
    }

    "address reviewer feedback in code and tests".to_owned()
}

fn normalize_review_text(value: &str) -> String {
    value
        .replace([',', ';', '\t', '\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_structured_review_text(value: &str) -> String {
    value
        .replace(['\t', '\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_review_text(value: &str, max_chars: usize) -> String {
    let mut text = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        text.push_str("...");
    }
    text
}
