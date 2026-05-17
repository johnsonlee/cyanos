//! Cyanos-owned verification and code-review judge boundary.

use std::{path::Path, process::Output};

use crate::{
    Agent, AgentExecutionMode, AgentRuntimeError, AppError, BenchmarkScore, CompositeScoreInput,
    CompositeScoreModel, Evaluation, EvaluationFinding, LlmJudgeScore, ModelSelection, QualityTier,
    SampleScore, ScoreBreakdown, TestScore,
};

const CARGO_CHECK_ARGS: &[&str] = &["check"];
const CARGO_CLIPPY_ARGS: &[&str] = &[
    "clippy",
    "--all-targets",
    "--all-features",
    "--",
    "-D",
    "warnings",
];
const CARGO_PROGRAM: &str = "cargo";
const CARGO_TEST_ARGS: &[&str] = &["test", "--all-targets", "--all-features"];
const EVIDENCE_BENCHMARK_NOT_MEASURED: &str = "benchmark: not_measured";
const EVIDENCE_JUDGE_NOT_MEASURED: &str = "judge: not_measured";
const EVIDENCE_REPOSITORY_CHANGED: &str = "repository_state: changed";
const EVIDENCE_REPOSITORY_UNCHANGED: &str = "repository_state: no_changes";
const FINDING_REPOSITORY_UNCHANGED: &str =
    "No source changes were produced; update the sample worktree before rerunning verification.";
const GIT_PROGRAM: &str = "git";
const GIT_STATUS_ARG: &str = "status";
const GIT_STATUS_PORCELAIN_ARG: &str = "--porcelain=v1";
const GIT_STATUS_UNTRACKED_ARG: &str = "--untracked-files=all";
const JUDGE_CATEGORY_REQUIREMENTS: &str = "requirements";
const JUDGE_FINDINGS_FIELD: &str = "findings=";
const JUDGE_FIX_DEFAULT: &str = "revise implementation against task brief and judge feedback";
const JUDGE_INVALID_OUTPUT_FIX: &str =
    "rerun judge with verdict rubric risk next findings and complete finding fields";
const JUDGE_MARKER: &str = "CYANOS_JUDGE";
const JUDGE_NEXT_DEFAULT: &str = "promote verified candidate or revise blocking findings";
const JUDGE_NEXT_FIELD: &str = "next=";
const JUDGE_RISK_FIELD: &str = "risk=";
const JUDGE_RISK_LOW: &str = "low";
const JUDGE_RUBRIC_FIELD: &str = "rubric=";
const JUDGE_RUBRIC_VERSION: &str = "mvp-2026-05-24";
const JUDGE_SCORE_MAX_U8: u8 = 100;
const JUDGE_SEVERITY_BLOCKING: &str = "blocking";
const JUDGE_SEVERITY_MAJOR_PENALTY: i64 = 5_000;
const JUDGE_SEVERITY_MINOR: &str = "minor";
const JUDGE_SEVERITY_MINOR_PENALTY: i64 = 2_500;
const JUDGE_SEVERITY_NOTE: &str = "note";
const JUDGE_SEVERITY_NOTE_PENALTY: i64 = 1_000;
const JUDGE_SUMMARY_FIELD: &str = "summary=";
const NEUTRAL_RECALL_BASIS_POINTS: i64 = 10_000;
const PERCENT_FRACTION_DIGITS: usize = 2;
const PERCENT_SCALE: i64 = 100;
const VERIFIER_CARGO_CHECK_LABEL: &str = "cargo_check";
const VERIFIER_CARGO_CLIPPY_LABEL: &str = "cargo_clippy";
const VERIFIER_CARGO_TEST_LABEL: &str = "cargo_test";
const VERIFIER_COMMAND_COUNT: usize = 3;

/// Independent judge system prompt used for code-review verification.
pub const JUDGE_SYSTEM_PROMPT: &str = "You are the independent Cyanos code review judge. Review only. Do not edit files. Decide whether the candidate satisfies the task brief and verifier evidence.";

/// Code-review penalty rule used to convert findings into recall score.
pub const JUDGE_RUBRIC_RULE: &str =
    "severity_penalty_v1 blocking=10000 major=5000 minor=2500 note=1000 category=worst_finding";

/// Basis-point divisor for formatted judge percentages.
pub const JUDGE_PERCENT_TO_BASIS_POINTS: i64 = 100;

/// Identity and isolation mode of the LLM judge adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JudgeIdentity {
    agent: Agent,
    model: ModelSelection,
    isolation_mode: &'static str,
}

impl JudgeIdentity {
    /// Selects the default isolated judge for a worker agent.
    #[must_use]
    pub fn for_worker(worker_agent: Agent) -> Self {
        Self {
            agent: worker_agent.default_judge_for_worker(),
            model: ModelSelection::BestSupported,
            isolation_mode: AgentExecutionMode::JudgeIsolated.as_label(),
        }
    }

    /// Returns the judge agent.
    #[must_use]
    pub const fn agent(&self) -> Agent {
        self.agent
    }

    /// Returns the judge model selection.
    #[must_use]
    pub const fn model(&self) -> &ModelSelection {
        &self.model
    }

    /// Returns the judge isolation label.
    #[must_use]
    pub const fn isolation_mode(&self) -> &'static str {
        self.isolation_mode
    }
}

/// Structured code-review judge finding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JudgeFinding {
    category: String,
    severity: String,
    concern: String,
    fix: String,
    evidence: String,
}

impl JudgeFinding {
    /// Parses one comma-separated judge finding segment.
    #[must_use]
    pub fn from_segment(segment: &str) -> Option<Self> {
        let mut category = None;
        let mut severity = None;
        let mut concern = None;
        let mut fix = None;
        let mut evidence = None;
        for part in segment.split(',') {
            let (key, value) = part.split_once('=')?;
            let normalized_value = value.trim();
            match key.trim() {
                "category" => category = Some(normalized_value.to_ascii_lowercase()),
                "severity" => severity = Some(normalized_value.to_ascii_lowercase()),
                "concern" => concern = Some(normalized_value.to_owned()),
                "fix" => fix = Some(normalized_value.to_owned()),
                "evidence" => evidence = Some(normalized_value.to_owned()),
                _ => {}
            }
        }
        let category = category?;
        let severity = severity?;
        let concern = concern?;
        let fix = fix?;
        let evidence = evidence?;
        Some(Self {
            category: non_empty_judge_field(&category)?.to_owned(),
            severity: non_empty_judge_field(&severity)?.to_owned(),
            concern: non_empty_judge_field(&concern)?.to_owned(),
            fix: non_empty_judge_field(&fix)?.to_owned(),
            evidence: non_empty_judge_field(&evidence)?.to_owned(),
        })
    }

    /// Creates a finding for invalid PR review comments.
    #[must_use]
    pub fn invalid_review_comment(concern: &str, fix: &str, evidence: &str) -> Self {
        Self {
            category: JUDGE_CATEGORY_REQUIREMENTS.to_owned(),
            severity: JUDGE_SEVERITY_BLOCKING.to_owned(),
            concern: concern.to_owned(),
            fix: fix.to_owned(),
            evidence: evidence.to_owned(),
        }
    }

    /// Adds provenance evidence to a parsed finding.
    #[must_use]
    pub fn with_evidence_suffix(mut self, suffix: &str) -> Self {
        self.evidence = format!("{} {}", self.evidence, suffix);
        self
    }

    /// Returns whether this finding blocks promotion.
    #[must_use]
    pub fn is_blocking(&self) -> bool {
        self.severity == JUDGE_SEVERITY_BLOCKING
    }

    /// Returns the severity penalty in basis points.
    #[must_use]
    pub fn penalty_basis_points(&self) -> i64 {
        if self.is_blocking() {
            return NEUTRAL_RECALL_BASIS_POINTS;
        }
        match self.severity.as_str() {
            JUDGE_SEVERITY_MINOR => JUDGE_SEVERITY_MINOR_PENALTY,
            JUDGE_SEVERITY_NOTE => JUDGE_SEVERITY_NOTE_PENALTY,
            _ => JUDGE_SEVERITY_MAJOR_PENALTY,
        }
    }

    /// Renders the finding as prompt-evolution feedback.
    #[must_use]
    pub fn to_message(&self) -> String {
        format!(
            "judge finding category={} severity={} concern={} fix={} evidence={}",
            self.category, self.severity, self.concern, self.fix, self.evidence
        )
    }

    /// Renders the finding as durable verifier evidence.
    #[must_use]
    pub fn to_evidence(&self) -> String {
        format!(
            "category={} severity={} concern={} fix={} evidence={}",
            self.category, self.severity, self.concern, self.fix, self.evidence
        )
    }

    /// Returns the finding category.
    #[must_use]
    pub fn category(&self) -> &str {
        &self.category
    }

    /// Returns the finding severity.
    #[must_use]
    pub fn severity(&self) -> &str {
        &self.severity
    }

    /// Returns the finding concern.
    #[must_use]
    pub fn concern(&self) -> &str {
        &self.concern
    }

    /// Returns the suggested fix.
    #[must_use]
    pub fn fix(&self) -> &str {
        &self.fix
    }

    /// Returns evidence for the finding.
    #[must_use]
    pub fn evidence_ref(&self) -> &str {
        &self.evidence
    }

    fn invalid_output() -> Self {
        Self {
            category: JUDGE_CATEGORY_REQUIREMENTS.to_owned(),
            severity: JUDGE_SEVERITY_BLOCKING.to_owned(),
            concern: "invalid judge output missing required rubric risk next or finding fix field"
                .to_owned(),
            fix: JUDGE_INVALID_OUTPUT_FIX.to_owned(),
            evidence: "judge".to_owned(),
        }
    }

    fn fallback_blocking(concern: &str) -> Self {
        Self {
            category: JUDGE_CATEGORY_REQUIREMENTS.to_owned(),
            severity: JUDGE_SEVERITY_BLOCKING.to_owned(),
            concern: concern.to_owned(),
            fix: JUDGE_FIX_DEFAULT.to_owned(),
            evidence: "judge".to_owned(),
        }
    }
}

/// Structured judge verdict and score.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JudgeReview {
    accepted: bool,
    score_basis_points: i64,
    judge_agent: String,
    judge_model: String,
    judge_isolation: String,
    rubric_version: String,
    risk_category: String,
    next_action: String,
    summary: String,
    findings: Vec<JudgeFinding>,
}

impl JudgeReview {
    /// Builds a passing judge review.
    #[must_use]
    pub fn passed(summary: &str, findings: Vec<JudgeFinding>) -> Self {
        Self::passed_with_metadata(
            JUDGE_RUBRIC_VERSION,
            JUDGE_RISK_LOW,
            JUDGE_NEXT_DEFAULT,
            summary,
            findings,
        )
    }

    /// Builds a passing judge review with explicit metadata.
    #[must_use]
    pub fn passed_with_metadata(
        rubric_version: &str,
        risk_category: &str,
        next_action: &str,
        summary: &str,
        findings: Vec<JudgeFinding>,
    ) -> Self {
        let score_basis_points = judge_recall_from_findings(&findings);
        Self {
            accepted: true,
            score_basis_points,
            judge_agent: "unconfigured".to_owned(),
            judge_model: "unconfigured".to_owned(),
            judge_isolation: "unconfigured".to_owned(),
            rubric_version: normalized_judge_metadata(rubric_version, JUDGE_RUBRIC_VERSION),
            risk_category: normalized_judge_metadata(risk_category, JUDGE_RISK_LOW),
            next_action: normalized_judge_metadata(next_action, JUDGE_NEXT_DEFAULT),
            summary: summary.to_owned(),
            findings,
        }
    }

    /// Builds a rejected judge review.
    #[must_use]
    pub fn rejected(summary: &str, findings: Vec<JudgeFinding>) -> Self {
        Self::rejected_with_metadata(
            JUDGE_RUBRIC_VERSION,
            JUDGE_RISK_LOW,
            JUDGE_FIX_DEFAULT,
            summary,
            findings,
        )
    }

    /// Builds a rejected judge review with explicit metadata.
    #[must_use]
    pub fn rejected_with_metadata(
        rubric_version: &str,
        risk_category: &str,
        next_action: &str,
        summary: &str,
        mut findings: Vec<JudgeFinding>,
    ) -> Self {
        if findings.is_empty() {
            findings.push(JudgeFinding::fallback_blocking(summary));
        }
        let score_basis_points = judge_recall_from_findings(&findings);
        Self {
            accepted: false,
            score_basis_points,
            judge_agent: "unconfigured".to_owned(),
            judge_model: "unconfigured".to_owned(),
            judge_isolation: "unconfigured".to_owned(),
            rubric_version: normalized_judge_metadata(rubric_version, JUDGE_RUBRIC_VERSION),
            risk_category: normalized_judge_metadata(risk_category, JUDGE_RISK_LOW),
            next_action: normalized_judge_metadata(next_action, JUDGE_FIX_DEFAULT),
            summary: summary.to_owned(),
            findings,
        }
    }

    /// Attaches judge adapter provenance.
    #[must_use]
    pub fn with_identity(mut self, identity: &JudgeIdentity) -> Self {
        identity.agent.as_str().clone_into(&mut self.judge_agent);
        identity.model.as_label().clone_into(&mut self.judge_model);
        identity
            .isolation_mode
            .clone_into(&mut self.judge_isolation);
        self
    }

    /// Returns whether the judge accepted the candidate.
    #[must_use]
    pub const fn accepted(&self) -> bool {
        self.accepted
    }

    /// Returns judge recall score in basis points.
    #[must_use]
    pub const fn score_basis_points(&self) -> i64 {
        self.score_basis_points
    }

    /// Returns structured judge findings.
    #[must_use]
    pub fn findings(&self) -> &[JudgeFinding] {
        &self.findings
    }

    /// Returns the review summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Renders judge evidence for the task evaluation snapshot.
    #[must_use]
    pub fn evidence(&self) -> String {
        let findings = if self.findings.is_empty() {
            "none".to_owned()
        } else {
            self.findings
                .iter()
                .map(JudgeFinding::to_evidence)
                .collect::<Vec<_>>()
                .join("; ")
        };
        format!(
            "judge: agent={} model={} isolation={} verdict={} rubric={} rule={} risk={} next={} recall={}.{:02}% findings=[{}] summary={}",
            self.judge_agent,
            self.judge_model,
            self.judge_isolation,
            if self.accepted { "pass" } else { "fail" },
            self.rubric_version,
            JUDGE_RUBRIC_RULE,
            self.risk_category,
            self.next_action,
            self.score_basis_points / 100,
            self.score_basis_points % JUDGE_PERCENT_TO_BASIS_POINTS,
            findings,
            self.summary
        )
    }
}

/// Result of running all verifier gates for a sample worktree.
#[derive(Debug)]
pub struct SampleVerification {
    evaluation: Evaluation,
    score: ScoreBreakdown,
}

impl SampleVerification {
    /// Creates a sample verification result.
    #[must_use]
    pub const fn new(evaluation: Evaluation, score: ScoreBreakdown) -> Self {
        Self { evaluation, score }
    }

    /// Consumes the result into evaluation and score values.
    #[must_use]
    pub fn into_parts(self) -> (Evaluation, ScoreBreakdown) {
        (self.evaluation, self.score)
    }

    /// Returns the verifier evaluation.
    #[must_use]
    pub const fn evaluation(&self) -> &Evaluation {
        &self.evaluation
    }

    /// Returns the composite score.
    #[must_use]
    pub const fn score(&self) -> &ScoreBreakdown {
        &self.score
    }
}

/// One structural verifier command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifierCommand {
    label: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

impl VerifierCommand {
    /// Returns the MVP structural verifier commands.
    #[must_use]
    pub const fn all() -> [Self; VERIFIER_COMMAND_COUNT] {
        [
            Self {
                label: VERIFIER_CARGO_CHECK_LABEL,
                program: CARGO_PROGRAM,
                args: CARGO_CHECK_ARGS,
            },
            Self {
                label: VERIFIER_CARGO_CLIPPY_LABEL,
                program: CARGO_PROGRAM,
                args: CARGO_CLIPPY_ARGS,
            },
            Self {
                label: VERIFIER_CARGO_TEST_LABEL,
                program: CARGO_PROGRAM,
                args: CARGO_TEST_ARGS,
            },
        ]
    }

    /// Returns the verifier label.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        self.label
    }

    /// Returns the program to execute.
    #[must_use]
    pub const fn program(&self) -> &'static str {
        self.program
    }

    /// Returns command arguments.
    #[must_use]
    pub const fn args(&self) -> &'static [&'static str] {
        self.args
    }
}

/// Runs Cyanos structural, recall, and judge verification for one sample.
///
/// # Errors
///
/// Returns an error when repository inspection, structural commands, coverage,
/// or judge execution cannot be completed.
#[expect(
    clippy::too_many_lines,
    reason = "verifier gate order is intentionally explicit"
)]
pub fn evaluate_sample_worktree_with_verifiers<R, M, J>(
    worktree: &Path,
    task_readme: &str,
    run_command: R,
    measure_coverage: M,
    judge_review: J,
) -> Result<SampleVerification, AppError>
where
    R: Copy + Fn(&str, &Path, &[&str], &'static str) -> Result<Output, AppError>,
    M: Fn(&Path) -> Result<i64, AppError>,
    J: Fn(&Path, &str) -> Result<JudgeReview, AppError>,
{
    let mut structural_evidence = Vec::new();
    let mut recall_evidence = pending_recall_evidence();
    let mut findings = Vec::new();
    let mut passed_structural_gates = 0_u32;
    let mut failed_structural_gates = 0_u32;
    let output = run_command(
        GIT_PROGRAM,
        worktree,
        &[
            GIT_STATUS_ARG,
            GIT_STATUS_PORCELAIN_ARG,
            GIT_STATUS_UNTRACKED_ARG,
        ],
        "run sample repository verifier",
    )?;

    if !output.status.success() {
        return Err(AppError::from(AgentRuntimeError::new(format!(
            "sample repository verifier failed in {} with exit code {:?}",
            worktree.display(),
            output.status.code()
        ))));
    }

    if output.stdout.is_empty() {
        structural_evidence.push(EVIDENCE_REPOSITORY_UNCHANGED.to_owned());
        failed_structural_gates += 1;
        return Ok(SampleVerification::new(
            Evaluation::rejected(vec![EvaluationFinding::new(
                FINDING_REPOSITORY_UNCHANGED.to_owned(),
            )])
            .with_evidence(structural_evidence, recall_evidence),
            score_breakdown(
                QualityTier::TestFailed,
                passed_structural_gates,
                failed_structural_gates,
                0,
                0,
            ),
        ));
    }

    structural_evidence.push(EVIDENCE_REPOSITORY_CHANGED.to_owned());
    passed_structural_gates += 1;
    for command in VerifierCommand::all() {
        let output = run_command(
            command.program(),
            worktree,
            command.args(),
            "run sample repository verifier",
        )?;
        if output.status.success() {
            structural_evidence.push(format!("{}: passed", command.label()));
            passed_structural_gates += 1;
        } else {
            structural_evidence.push(format!(
                "{}: failed exit_code={:?}",
                command.label(),
                output.status.code()
            ));
            failed_structural_gates += 1;
            findings.push(EvaluationFinding::new(format!(
                "{} failed; fix verifier failures before promotion.",
                command.label()
            )));
            let tier = if command.label() == VERIFIER_CARGO_CHECK_LABEL {
                QualityTier::CompileFailed
            } else {
                QualityTier::TestFailed
            };
            return Ok(SampleVerification::new(
                Evaluation::rejected(findings).with_evidence(structural_evidence, recall_evidence),
                score_breakdown(tier, passed_structural_gates, failed_structural_gates, 0, 0),
            ));
        }
    }

    let coverage = measure_coverage(worktree)?;
    recall_evidence.push(format_coverage_evidence(coverage));
    if coverage == 0 {
        findings.push(EvaluationFinding::new(
            "coverage recall is zero; add or repair tests before promotion.".to_owned(),
        ));
        return Ok(SampleVerification::new(
            Evaluation::rejected(findings).with_evidence(structural_evidence, recall_evidence),
            score_breakdown(
                QualityTier::PartialSuccess,
                passed_structural_gates,
                failed_structural_gates,
                coverage,
                0,
            ),
        ));
    }

    let diff = worktree_diff(worktree, run_command)?;
    let judge_prompt =
        judge_review_prompt(task_readme, &diff, &structural_evidence, &recall_evidence);
    let judge = judge_review(worktree, &judge_prompt)?;
    recall_evidence.push(judge.evidence());
    if !judge.accepted() {
        findings.extend(
            judge
                .findings()
                .iter()
                .map(|finding| EvaluationFinding::new(finding.to_message())),
        );
        return Ok(SampleVerification::new(
            Evaluation::rejected(findings).with_evidence(structural_evidence, recall_evidence),
            score_breakdown(
                QualityTier::PartialSuccess,
                passed_structural_gates,
                failed_structural_gates,
                coverage,
                judge.score_basis_points(),
            ),
        ));
    }

    Ok(SampleVerification::new(
        Evaluation::accepted_with_findings(findings)
            .with_evidence(structural_evidence, recall_evidence),
        score_breakdown(
            QualityTier::Passed,
            passed_structural_gates,
            failed_structural_gates,
            coverage,
            judge.score_basis_points(),
        ),
    ))
}

/// Parses a coverage percentage into basis points.
#[must_use]
pub fn parse_coverage_basis_points(output: &str) -> Option<i64> {
    output
        .lines()
        .filter(|line| line.contains("TOTAL"))
        .filter_map(|line| {
            line.split_whitespace()
                .filter_map(|part| part.strip_suffix('%'))
                .next_back()
                .and_then(parse_percent_basis_points)
        })
        .next_back()
}

/// Formats coverage recall evidence.
#[must_use]
pub fn format_coverage_evidence(coverage_basis_points: i64) -> String {
    let coverage = coverage_basis_points.clamp(0, NEUTRAL_RECALL_BASIS_POINTS);
    format!("coverage: {}.{:02}%", coverage / 100, coverage % 100)
}

/// Builds the isolated code-review judge prompt.
#[must_use]
pub fn judge_review_prompt(
    task_readme: &str,
    diff: &str,
    structural_evidence: &[String],
    recall_evidence: &[String],
) -> String {
    format!(
        "Cyanos code review judge\n\nTask brief:\n{}\n\nCandidate diff:\n```diff\n{}\n```\n\nVerifier evidence:\nStructural:\n{}\nRecall:\n{}\n\nReturn exactly one final line. Do not request or perform GitHub comment operations. Cyanos owns comments. Use comma-separated finding fields and semicolon-separated findings; do not use comma or semicolon inside field values. Every finding must include a fix suggestion:\n{JUDGE_MARKER} verdict=<pass|fail> rubric={JUDGE_RUBRIC_VERSION} risk=<low|medium|high|critical> next=<safe next action> findings=<none|category=<requirements|correctness|security|reliability|tests|maintainability|docs>,severity=<blocking|major|minor|note>,concern=<concise concern>,fix=<safe next action>,evidence=<diff or verifier reference>[;...]> summary=<short verdict>",
        task_readme.trim(),
        diff,
        structural_evidence.join("\n"),
        recall_evidence.join("\n")
    )
}

/// Parses the final judge line from agent output.
#[must_use]
pub fn parse_judge_review(output: &str) -> JudgeReview {
    let Some(line) = output
        .lines()
        .rev()
        .find(|line| line.contains(JUDGE_MARKER))
    else {
        return JudgeReview::rejected("judge did not emit CYANOS_JUDGE verdict", Vec::new());
    };
    let verdict = field_value(line, "verdict=").unwrap_or_default();
    let rubric_version = field_value(line, JUDGE_RUBRIC_FIELD).unwrap_or_default();
    let risk_category = field_value(line, JUDGE_RISK_FIELD).unwrap_or_default();
    let next_action = field_segment(
        line,
        JUDGE_NEXT_FIELD,
        &[JUDGE_FINDINGS_FIELD, JUDGE_SUMMARY_FIELD],
    )
    .unwrap_or_default();
    let summary = field_remainder(line, JUDGE_SUMMARY_FIELD).unwrap_or("no summary");
    let findings_segment = field_segment(line, JUDGE_FINDINGS_FIELD, &[JUDGE_SUMMARY_FIELD]);
    let parsed_findings =
        findings_segment.map_or_else(JudgeFindingsParse::missing, parse_judge_findings);
    let mut findings = parsed_findings.findings;
    if missing_judge_schema_field(rubric_version)
        || missing_judge_schema_field(risk_category)
        || missing_judge_schema_field(next_action)
        || parsed_findings.invalid_segments > 0
    {
        findings.push(JudgeFinding::invalid_output());
    }
    let accepted = matches!(
        verdict.to_ascii_lowercase().as_str(),
        "pass" | "passed" | "accept" | "accepted"
    );
    if accepted && findings.iter().all(|finding| !finding.is_blocking()) {
        JudgeReview::passed_with_metadata(
            rubric_version,
            risk_category,
            next_action,
            summary,
            findings,
        )
    } else {
        JudgeReview::rejected_with_metadata(
            rubric_version,
            risk_category,
            next_action,
            summary,
            findings,
        )
    }
}

/// Converts judge findings to multiplicative code-review recall.
#[must_use]
pub fn judge_recall_from_findings(findings: &[JudgeFinding]) -> i64 {
    let mut category_penalties: Vec<(String, i64)> = Vec::new();
    for finding in findings {
        let penalty = finding
            .penalty_basis_points()
            .clamp(0, NEUTRAL_RECALL_BASIS_POINTS);
        if let Some((_category, existing)) = category_penalties
            .iter_mut()
            .find(|(category, _penalty)| category == &finding.category)
        {
            *existing = (*existing).max(penalty);
        } else {
            category_penalties.push((finding.category.clone(), penalty));
        }
    }

    category_penalties.into_iter().fold(
        NEUTRAL_RECALL_BASIS_POINTS,
        |score, (_category, penalty)| {
            let category_score = NEUTRAL_RECALL_BASIS_POINTS - penalty;
            score * category_score / NEUTRAL_RECALL_BASIS_POINTS
        },
    )
}

fn pending_recall_evidence() -> Vec<String> {
    vec![
        EVIDENCE_BENCHMARK_NOT_MEASURED.to_owned(),
        EVIDENCE_JUDGE_NOT_MEASURED.to_owned(),
    ]
}

fn score_breakdown(
    quality_tier: QualityTier,
    passed_structural_gates: u32,
    failed_structural_gates: u32,
    coverage_basis_points: i64,
    judge_basis_points: i64,
) -> ScoreBreakdown {
    let structural_score = TestScore::new(passed_structural_gates, failed_structural_gates);
    let judge_score = u8::try_from(
        judge_basis_points.clamp(0, NEUTRAL_RECALL_BASIS_POINTS) / JUDGE_PERCENT_TO_BASIS_POINTS,
    )
    .unwrap_or(JUDGE_SCORE_MAX_U8);
    let score = CompositeScoreModel::new().score(CompositeScoreInput::new(
        quality_tier,
        structural_score,
        BenchmarkScore::new(coverage_basis_points),
        LlmJudgeScore::new(judge_score),
    ));
    let structural_basis_points = if passed_structural_gates + failed_structural_gates == 0 {
        0
    } else {
        i64::from(passed_structural_gates) * NEUTRAL_RECALL_BASIS_POINTS
            / i64::from(passed_structural_gates + failed_structural_gates)
    };

    ScoreBreakdown::new(
        score,
        quality_tier,
        SampleScore::new(structural_basis_points),
        SampleScore::new(coverage_basis_points),
        SampleScore::new(judge_basis_points.clamp(0, NEUTRAL_RECALL_BASIS_POINTS)),
    )
}

fn worktree_diff<R>(worktree: &Path, run_command: R) -> Result<String, AppError>
where
    R: Fn(&str, &Path, &[&str], &'static str) -> Result<Output, AppError>,
{
    let output = run_command(
        GIT_PROGRAM,
        worktree,
        &["diff", "--no-ext-diff", "--", "."],
        "collect candidate diff for judge",
    )?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(AgentRuntimeError::new(format!(
            "git diff failed before judge in {} with exit code {:?}",
            worktree.display(),
            output.status.code()
        ))
        .into())
    }
}

fn parse_percent_basis_points(value: &str) -> Option<i64> {
    let mut parts = value.split('.');
    let whole = parts.next()?.parse::<i64>().ok()?;
    let fraction = parts.next().unwrap_or("0");
    if parts.next().is_some() {
        return None;
    }
    let mut fraction = fraction
        .chars()
        .take(PERCENT_FRACTION_DIGITS)
        .collect::<String>();
    while fraction.len() < PERCENT_FRACTION_DIGITS {
        fraction.push('0');
    }
    let fraction = fraction.parse::<i64>().ok()?;
    Some((whole * PERCENT_SCALE + fraction).clamp(0, NEUTRAL_RECALL_BASIS_POINTS))
}

fn field_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.split_once(key)?.1;
    rest.split_whitespace().next()
}

fn field_remainder<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_once(key)
        .map(|(_prefix, rest)| rest.trim())
        .filter(|value| !value.is_empty())
}

fn field_segment<'a>(line: &'a str, key: &str, next_keys: &[&str]) -> Option<&'a str> {
    let rest = line.split_once(key)?.1;
    let end = next_keys
        .iter()
        .filter_map(|next_key| rest.find(next_key))
        .min()
        .unwrap_or(rest.len());
    Some(rest[..end].trim()).filter(|value| !value.is_empty())
}

fn missing_judge_schema_field(value: &str) -> bool {
    value.trim().is_empty()
}

fn parse_judge_findings(value: &str) -> JudgeFindingsParse {
    if value.eq_ignore_ascii_case("none") {
        return JudgeFindingsParse::default();
    }
    let mut parsed = JudgeFindingsParse::default();
    for segment in value
        .split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
    {
        if let Some(finding) = JudgeFinding::from_segment(segment) {
            parsed.findings.push(finding);
        } else {
            parsed.invalid_segments += 1;
        }
    }
    parsed
}

fn normalized_judge_metadata(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn non_empty_judge_field(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct JudgeFindingsParse {
    findings: Vec<JudgeFinding>,
    invalid_segments: usize,
}

impl JudgeFindingsParse {
    fn missing() -> Self {
        Self {
            findings: Vec::new(),
            invalid_segments: 1,
        }
    }
}
