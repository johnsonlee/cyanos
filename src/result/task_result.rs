//! Task-level result state written to `tasks/<task-id>/result.json`.

use crate::result::{
    ScoreBreakdown, ScoreTransition, TaskResultStatus, TaskRunRecord, VerifierFeedback,
};
use crate::{QualityTier, SampleScore};

const RESULT_FIELD_BASELINE_SCORE: &str = "\"baseline_score\"";
const RESULT_FIELD_BENCHMARK: &str = "\"benchmark\"";
const RESULT_FIELD_BENCHMARK_SUMMARY: &str = "\"benchmark_summary\"";
const RESULT_FIELD_BEST_SCORE: &str = "\"best_score\"";
const RESULT_FIELD_CURRENT_SCORE: &str = "\"current_score\"";
const RESULT_FIELD_EVAL_PATH: &str = "\"eval_path\"";
const RESULT_FIELD_FAILURE_CLASS: &str = "\"failure_class\"";
const RESULT_FIELD_FEEDBACK: &str = "\"feedback\"";
const RESULT_FIELD_FINDINGS: &str = "\"findings\"";
const RESULT_FIELD_JUDGE: &str = "\"judge\"";
const RESULT_FIELD_NEXT_ACTION: &str = "\"next_action\"";
const RESULT_FIELD_PERFECT_SCORE: &str = "\"perfect_score\"";
const RESULT_FIELD_PREVIOUS_SCORE: &str = "\"previous_score\"";
const RESULT_FIELD_QUALITY_TIER: &str = "\"quality_tier\"";
const RESULT_FIELD_RUN_INDEX: &str = "\"run_index\"";
const RESULT_FIELD_SELECTED_SAMPLE: &str = "\"selected_sample\"";
const RESULT_FIELD_SELECTED_SCORE: &str = "\"selected_score\"";
const RESULT_FIELD_STATUS: &str = "\"status\"";
const RESULT_FIELD_SUMMARY: &str = "\"summary\"";
const RESULT_FIELD_TESTS: &str = "\"tests\"";
const RESULT_FIELD_TOTAL: &str = "\"total\"";
const RESULT_TOP_LEVEL_STATUS_PREFIX: &str = "  \"status\"";
const JSON_COLON: char = ':';
const JSON_COMMA: char = ',';
const JSON_ESCAPE: char = '\\';
const JSON_QUOTE: char = '"';
const JSON_UNICODE_ESCAPE: char = 'u';
const UNICODE_ESCAPE_DIGITS: usize = 4;
const UNICODE_ESCAPE_RADIX: u32 = 16;

/// Task-level result state written to `tasks/<task-id>/result.json`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskResult {
    baseline_score: ScoreBreakdown,
    current_score: ScoreBreakdown,
    best_score: ScoreBreakdown,
    perfect_score: ScoreBreakdown,
    status: TaskResultStatus,
    runs: Vec<TaskRunRecord>,
}

impl TaskResult {
    /// Creates an empty task result at the baseline score.
    #[must_use]
    pub const fn new(baseline_score: ScoreBreakdown, perfect_score: ScoreBreakdown) -> Self {
        Self {
            baseline_score,
            current_score: baseline_score,
            best_score: baseline_score,
            perfect_score,
            status: TaskResultStatus::Baseline,
            runs: Vec::new(),
        }
    }

    /// Parses a rendered task result JSON artifact into executable task state.
    ///
    /// # Errors
    ///
    /// Returns an error when required score, status, run, or feedback fields are
    /// missing or malformed.
    pub fn from_json(content: &str) -> Result<Self, TaskResultParseError> {
        let lines = content.lines().collect::<Vec<_>>();
        let baseline_score = parse_score_object(&lines, RESULT_FIELD_BASELINE_SCORE)?;
        let current_score = parse_score_object(&lines, RESULT_FIELD_CURRENT_SCORE)?;
        let best_score = parse_score_object(&lines, RESULT_FIELD_BEST_SCORE)?;
        let perfect_score = parse_score_object(&lines, RESULT_FIELD_PERFECT_SCORE)?;
        let status = content
            .lines()
            .find(|line| line.starts_with(RESULT_TOP_LEVEL_STATUS_PREFIX))
            .and_then(|line| json_string_field(line, RESULT_FIELD_STATUS))
            .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_STATUS))
            .and_then(|label| parse_task_result_status(&label))?;
        let runs = parse_run_records(&lines, perfect_score)?;

        Ok(Self {
            baseline_score,
            current_score,
            best_score,
            perfect_score,
            status,
            runs,
        })
    }

    /// Records one outer run in the task-level result.
    pub fn record_run(
        &mut self,
        run_index: usize,
        selected_sample: usize,
        selected_score: ScoreBreakdown,
        feedback: VerifierFeedback,
    ) {
        let previous_score = self.current_score;
        let score = ScoreTransition::new(previous_score, selected_score, self.perfect_score);
        let record = TaskRunRecord::new(run_index, selected_sample, score, feedback);

        self.current_score = selected_score;
        if selected_score.total().as_i64() > self.best_score.total().as_i64() {
            self.best_score = selected_score;
        }
        self.status =
            TaskResultStatus::classify(self.baseline_score, self.current_score, self.perfect_score);
        self.runs.push(record);
    }

    /// Returns the task baseline score.
    #[must_use]
    pub const fn baseline_score(&self) -> ScoreBreakdown {
        self.baseline_score
    }

    /// Returns the latest selected score.
    #[must_use]
    pub const fn current_score(&self) -> ScoreBreakdown {
        self.current_score
    }

    /// Returns the highest score seen by this task.
    #[must_use]
    pub const fn best_score(&self) -> ScoreBreakdown {
        self.best_score
    }

    /// Returns the threshold score that marks the task perfect.
    #[must_use]
    pub const fn perfect_score(&self) -> ScoreBreakdown {
        self.perfect_score
    }

    /// Returns the current task-level status.
    #[must_use]
    pub const fn status(&self) -> TaskResultStatus {
        self.status
    }

    /// Returns per-run score records.
    #[must_use]
    pub fn runs(&self) -> &[TaskRunRecord] {
        &self.runs
    }

    /// Renders the task result as a runtime JSON artifact.
    #[must_use]
    pub fn to_json(&self) -> String {
        let runs = self
            .runs()
            .iter()
            .map(|run| run.render_json("    "))
            .collect::<Vec<_>>()
            .join(",\n");

        format!(
            "{{\n  \"baseline_score\": {},\n  \"current_score\": {},\n  \"best_score\": {},\n  \"perfect_score\": {},\n  \"status\": {},\n  \"runs\": [\n{}\n  ]\n}}\n",
            self.baseline_score().render_json("  "),
            self.current_score().render_json("  "),
            self.best_score().render_json("  "),
            self.perfect_score().render_json("  "),
            crate::json::string(self.status().as_str()),
            runs
        )
    }
}

/// Typed summary parsed from a task-level `result.json` runtime artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskResultSnapshot {
    status: String,
    runs: Vec<TaskRunSnapshot>,
}

impl TaskResultSnapshot {
    /// Parses the durable `result.json` artifact into typed resume state.
    ///
    /// # Errors
    ///
    /// Returns an error when required fields are missing or malformed.
    pub fn parse(content: &str) -> Result<Self, TaskResultParseError> {
        let status = content
            .lines()
            .find(|line| line.starts_with(RESULT_TOP_LEVEL_STATUS_PREFIX))
            .and_then(|line| json_string_field(line, RESULT_FIELD_STATUS))
            .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_STATUS))?;
        let runs = parse_run_snapshots(content)?;
        Ok(Self { status, runs })
    }

    /// Returns the top-level task result status label.
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Returns parsed run summaries.
    #[must_use]
    pub fn runs(&self) -> &[TaskRunSnapshot] {
        &self.runs
    }

    /// Returns the highest parsed run index.
    #[must_use]
    pub fn max_run_index(&self) -> Option<usize> {
        self.runs().iter().map(TaskRunSnapshot::run_index).max()
    }

    /// Returns true when the snapshot records this run/sample/eval selection.
    #[must_use]
    pub fn has_selected_sample_eval(
        &self,
        run_index: usize,
        selected_sample: usize,
        eval_path: &str,
    ) -> bool {
        self.runs().iter().any(|run| {
            run.run_index() == run_index
                && run.selected_sample() == selected_sample
                && run.eval_path() == eval_path
        })
    }
}

/// Typed summary for one run entry in `result.json`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRunSnapshot {
    run_index: usize,
    selected_sample: usize,
    status: String,
    eval_path: String,
    next_action: String,
}

impl TaskRunSnapshot {
    /// Returns the outer run index.
    #[must_use]
    pub const fn run_index(&self) -> usize {
        self.run_index
    }

    /// Returns the selected sample index.
    #[must_use]
    pub const fn selected_sample(&self) -> usize {
        self.selected_sample
    }

    /// Returns the run status label.
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Returns the selected sample eval artifact path.
    #[must_use]
    pub fn eval_path(&self) -> &str {
        &self.eval_path
    }

    /// Returns the recorded next action.
    #[must_use]
    pub fn next_action(&self) -> &str {
        &self.next_action
    }
}

#[derive(Default)]
struct TaskRunSnapshotBuilder {
    run_index: Option<usize>,
    selected_sample: Option<usize>,
    status: Option<String>,
    eval_path: Option<String>,
    next_action: Option<String>,
}

#[derive(Default)]
struct TaskRunRecordBuilder {
    run_index: Option<usize>,
    selected_sample: Option<usize>,
    previous_score: Option<ScoreBreakdown>,
    selected_score: Option<ScoreBreakdown>,
    feedback: Option<VerifierFeedback>,
}

impl TaskRunRecordBuilder {
    fn finish(self, perfect_score: ScoreBreakdown) -> Result<TaskRunRecord, TaskResultParseError> {
        let previous_score = self
            .previous_score
            .ok_or(TaskResultParseError::MissingField(
                RESULT_FIELD_PREVIOUS_SCORE,
            ))?;
        let selected_score = self
            .selected_score
            .ok_or(TaskResultParseError::MissingField(
                RESULT_FIELD_SELECTED_SCORE,
            ))?;
        Ok(TaskRunRecord::new(
            self.run_index
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_RUN_INDEX))?,
            self.selected_sample
                .ok_or(TaskResultParseError::MissingField(
                    RESULT_FIELD_SELECTED_SAMPLE,
                ))?,
            ScoreTransition::new(previous_score, selected_score, perfect_score),
            self.feedback
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_FEEDBACK))?,
        ))
    }
}

#[derive(Default)]
struct VerifierFeedbackBuilder {
    eval_path: Option<String>,
    summary: Option<String>,
    failure_class: Option<String>,
    findings: Option<Vec<String>>,
    benchmark_summary: Option<String>,
    next_action: Option<String>,
}

impl VerifierFeedbackBuilder {
    fn finish(self) -> Result<VerifierFeedback, TaskResultParseError> {
        Ok(VerifierFeedback::new(
            self.eval_path
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_EVAL_PATH))?,
            self.summary
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_SUMMARY))?,
            self.failure_class
                .ok_or(TaskResultParseError::MissingField(
                    RESULT_FIELD_FAILURE_CLASS,
                ))?,
        )
        .with_findings(
            self.findings
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_FINDINGS))?,
        )
        .with_benchmark_summary(self.benchmark_summary.ok_or(
            TaskResultParseError::MissingField(RESULT_FIELD_BENCHMARK_SUMMARY),
        )?)
        .with_next_action(
            self.next_action
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_NEXT_ACTION))?,
        ))
    }
}

impl TaskRunSnapshotBuilder {
    fn finish(self) -> Result<TaskRunSnapshot, TaskResultParseError> {
        Ok(TaskRunSnapshot {
            run_index: self
                .run_index
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_RUN_INDEX))?,
            selected_sample: self
                .selected_sample
                .ok_or(TaskResultParseError::MissingField(
                    RESULT_FIELD_SELECTED_SAMPLE,
                ))?,
            status: self
                .status
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_STATUS))?,
            eval_path: self
                .eval_path
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_EVAL_PATH))?,
            next_action: self
                .next_action
                .ok_or(TaskResultParseError::MissingField(RESULT_FIELD_NEXT_ACTION))?,
        })
    }
}

fn parse_run_snapshots(content: &str) -> Result<Vec<TaskRunSnapshot>, TaskResultParseError> {
    let mut runs = Vec::new();
    let mut current = None::<TaskRunSnapshotBuilder>;

    for line in content.lines() {
        if let Some(run_index) = json_usize_field(line, RESULT_FIELD_RUN_INDEX)? {
            if let Some(builder) = current.take() {
                runs.push(builder.finish()?);
            }
            current = Some(TaskRunSnapshotBuilder {
                run_index: Some(run_index),
                ..TaskRunSnapshotBuilder::default()
            });
            continue;
        }

        let Some(builder) = current.as_mut() else {
            continue;
        };
        if let Some(selected_sample) = json_usize_field(line, RESULT_FIELD_SELECTED_SAMPLE)? {
            builder.selected_sample = Some(selected_sample);
        } else if let Some(status) = json_string_field(line, RESULT_FIELD_STATUS) {
            builder.status = Some(status);
        } else if let Some(eval_path) = json_string_field(line, RESULT_FIELD_EVAL_PATH) {
            builder.eval_path = Some(eval_path);
        } else if let Some(next_action) = json_string_field(line, RESULT_FIELD_NEXT_ACTION) {
            builder.next_action = Some(next_action);
        }
    }

    if let Some(builder) = current.take() {
        runs.push(builder.finish()?);
    }

    Ok(runs)
}

fn parse_run_records(
    lines: &[&str],
    perfect_score: ScoreBreakdown,
) -> Result<Vec<TaskRunRecord>, TaskResultParseError> {
    let starts = run_record_starts(lines)?;
    let mut runs = Vec::new();

    for (position, start) in starts.iter().copied().enumerate() {
        let end = starts
            .get(position.saturating_add(1))
            .copied()
            .unwrap_or(lines.len());
        let builder = TaskRunRecordBuilder {
            run_index: Some(required_usize_in_range(
                lines,
                start,
                end,
                RESULT_FIELD_RUN_INDEX,
            )?),
            selected_sample: Some(required_usize_in_range(
                lines,
                start,
                end,
                RESULT_FIELD_SELECTED_SAMPLE,
            )?),
            previous_score: Some(parse_score_object_in_range(
                lines,
                start,
                end,
                RESULT_FIELD_PREVIOUS_SCORE,
            )?),
            selected_score: Some(parse_score_object_in_range(
                lines,
                start,
                end,
                RESULT_FIELD_SELECTED_SCORE,
            )?),
            feedback: Some(parse_feedback_object_in_range(
                lines,
                start,
                end,
                RESULT_FIELD_FEEDBACK,
            )?),
        };
        runs.push(builder.finish(perfect_score)?);
    }

    Ok(runs)
}

fn run_record_starts(lines: &[&str]) -> Result<Vec<usize>, TaskResultParseError> {
    let mut starts = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if json_field_value(line, RESULT_FIELD_RUN_INDEX).is_some() {
            json_usize_field(line, RESULT_FIELD_RUN_INDEX)?;
            starts.push(index);
        }
    }
    Ok(starts)
}

fn required_usize_in_range(
    lines: &[&str],
    start: usize,
    end: usize,
    field: &'static str,
) -> Result<usize, TaskResultParseError> {
    for line in lines
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(_, line)| *line)
    {
        if let Some(value) = json_usize_field(line, field)? {
            return Ok(value);
        }
    }
    Err(TaskResultParseError::MissingField(field))
}

fn parse_score_object(
    lines: &[&str],
    field: &'static str,
) -> Result<ScoreBreakdown, TaskResultParseError> {
    parse_score_object_in_range(lines, 0, lines.len(), field)
}

fn parse_score_object_in_range(
    lines: &[&str],
    start: usize,
    end: usize,
    field: &'static str,
) -> Result<ScoreBreakdown, TaskResultParseError> {
    let object_start = lines
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .find_map(|(index, line)| line.contains(field).then_some(index))
        .ok_or(TaskResultParseError::MissingField(field))?;
    let mut total = None;
    let mut quality_tier = None;
    let mut tests = None;
    let mut benchmark = None;
    let mut judge = None;

    for line in lines
        .iter()
        .enumerate()
        .skip(object_start.saturating_add(1))
        .take(end.saturating_sub(object_start.saturating_add(1)))
        .map(|(_, line)| *line)
    {
        if line.trim_start().starts_with('}') {
            break;
        }
        if let Some(value) = json_i64_field(line, RESULT_FIELD_TOTAL)? {
            total = Some(value);
        } else if let Some(value) = json_string_field(line, RESULT_FIELD_QUALITY_TIER) {
            quality_tier = Some(parse_quality_tier(&value)?);
        } else if let Some(value) = json_i64_field(line, RESULT_FIELD_TESTS)? {
            tests = Some(value);
        } else if let Some(value) = json_i64_field(line, RESULT_FIELD_BENCHMARK)? {
            benchmark = Some(value);
        } else if let Some(value) = json_i64_field(line, RESULT_FIELD_JUDGE)? {
            judge = Some(value);
        }
    }

    Ok(ScoreBreakdown::new(
        SampleScore::new(total.ok_or(TaskResultParseError::MissingField(RESULT_FIELD_TOTAL))?),
        quality_tier.ok_or(TaskResultParseError::MissingField(
            RESULT_FIELD_QUALITY_TIER,
        ))?,
        SampleScore::new(tests.ok_or(TaskResultParseError::MissingField(RESULT_FIELD_TESTS))?),
        SampleScore::new(
            benchmark.ok_or(TaskResultParseError::MissingField(RESULT_FIELD_BENCHMARK))?,
        ),
        SampleScore::new(judge.ok_or(TaskResultParseError::MissingField(RESULT_FIELD_JUDGE))?),
    ))
}

fn parse_feedback_object_in_range(
    lines: &[&str],
    start: usize,
    end: usize,
    field: &'static str,
) -> Result<VerifierFeedback, TaskResultParseError> {
    let object_start = lines
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .find_map(|(index, line)| line.contains(field).then_some(index))
        .ok_or(TaskResultParseError::MissingField(field))?;
    let mut builder = VerifierFeedbackBuilder::default();

    for line in lines
        .iter()
        .enumerate()
        .skip(object_start.saturating_add(1))
        .take(end.saturating_sub(object_start.saturating_add(1)))
        .map(|(_, line)| *line)
    {
        if line.trim_start().starts_with('}') {
            break;
        }
        if let Some(value) = json_string_field(line, RESULT_FIELD_EVAL_PATH) {
            builder.eval_path = Some(value);
        } else if let Some(value) = json_string_field(line, RESULT_FIELD_SUMMARY) {
            builder.summary = Some(value);
        } else if let Some(value) = json_string_field(line, RESULT_FIELD_FAILURE_CLASS) {
            builder.failure_class = Some(value);
        } else if let Some(value) = json_string_array_field(line, RESULT_FIELD_FINDINGS) {
            builder.findings = Some(value);
        } else if let Some(value) = json_string_field(line, RESULT_FIELD_BENCHMARK_SUMMARY) {
            builder.benchmark_summary = Some(value);
        } else if let Some(value) = json_string_field(line, RESULT_FIELD_NEXT_ACTION) {
            builder.next_action = Some(value);
        }
    }

    builder.finish()
}

fn parse_quality_tier(value: &str) -> Result<QualityTier, TaskResultParseError> {
    match value {
        "compile_failed" => Ok(QualityTier::CompileFailed),
        "test_failed" => Ok(QualityTier::TestFailed),
        "partial_success" => Ok(QualityTier::PartialSuccess),
        "passed" => Ok(QualityTier::Passed),
        _ => Err(TaskResultParseError::InvalidQualityTier(value.to_owned())),
    }
}

fn parse_task_result_status(value: &str) -> Result<TaskResultStatus, TaskResultParseError> {
    match value {
        "baseline" => Ok(TaskResultStatus::Baseline),
        "improved" => Ok(TaskResultStatus::Improved),
        "regressed" => Ok(TaskResultStatus::Regressed),
        "perfect" => Ok(TaskResultStatus::Perfect),
        _ => Err(TaskResultParseError::InvalidStatus(value.to_owned())),
    }
}

fn json_usize_field(
    line: &str,
    field: &'static str,
) -> Result<Option<usize>, TaskResultParseError> {
    let Some(value) = json_field_value(line, field) else {
        return Ok(None);
    };
    value
        .trim_end_matches(JSON_COMMA)
        .trim()
        .parse()
        .map(Some)
        .map_err(|_source| TaskResultParseError::InvalidInteger(field))
}

fn json_i64_field(line: &str, field: &'static str) -> Result<Option<i64>, TaskResultParseError> {
    let Some(value) = json_field_value(line, field) else {
        return Ok(None);
    };
    value
        .trim_end_matches(JSON_COMMA)
        .trim()
        .parse()
        .map(Some)
        .map_err(|_source| TaskResultParseError::InvalidInteger(field))
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

/// Error returned when parsing `result.json` as typed runtime state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskResultParseError {
    /// A required runtime field is missing.
    MissingField(&'static str),
    /// An integer field could not be parsed.
    InvalidInteger(&'static str),
    /// A persisted quality tier is unknown.
    InvalidQualityTier(String),
    /// A persisted task result status is unknown.
    InvalidStatus(String),
}

impl std::fmt::Display for TaskResultParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing task result field {field}"),
            Self::InvalidInteger(field) => write!(f, "invalid task result integer {field}"),
            Self::InvalidQualityTier(value) => write!(f, "invalid quality tier {value}"),
            Self::InvalidStatus(value) => write!(f, "invalid task result status {value}"),
        }
    }
}

impl std::error::Error for TaskResultParseError {}

#[cfg(test)]
mod tests {
    use crate::{
        QualityTier, SampleScore, ScoreBreakdown, TaskResult, TaskResultParseError,
        TaskResultSnapshot, TaskResultStatus, TaskRunRecord, VerifierFeedback,
    };

    fn score(
        total: i64,
        tier: QualityTier,
        tests: i64,
        benchmark: i64,
        judge: i64,
    ) -> ScoreBreakdown {
        ScoreBreakdown::new(
            SampleScore::new(total),
            tier,
            SampleScore::new(tests),
            SampleScore::new(benchmark),
            SampleScore::new(judge),
        )
    }

    fn feedback(action: &str) -> VerifierFeedback {
        VerifierFeedback::new(
            "runs/1/samples/1/eval.json".to_owned(),
            "tests passed".to_owned(),
            "none".to_owned(),
        )
        .with_findings(vec!["coverage ok".to_owned()])
        .with_benchmark_summary("no regression".to_owned())
        .with_next_action(action.to_owned())
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture asserts run existence")]
    fn records_task_runs_and_status_transitions() {
        let baseline = score(10, QualityTier::TestFailed, 1, 0, 0);
        let perfect = score(100, QualityTier::Passed, 10, 10, 10);
        let mut result = TaskResult::new(baseline, perfect);

        result.record_run(1, 2, baseline, feedback("continue"));
        result.record_run(
            2,
            1,
            score(15, QualityTier::PartialSuccess, 3, 1, 1),
            feedback("promote"),
        );
        result.record_run(
            3,
            1,
            score(12, QualityTier::PartialSuccess, 2, 0, 1),
            feedback("revise prompt"),
        );

        assert_eq!(result.baseline_score().total().as_i64(), 10);
        assert_eq!(result.current_score().total().as_i64(), 12);
        assert_eq!(result.best_score().total().as_i64(), 15);
        assert_eq!(result.best_score().tests().as_i64(), 3);
        assert_eq!(result.perfect_score().total().as_i64(), 100);
        assert_eq!(result.status(), TaskResultStatus::Improved);
        assert_eq!(result.runs().len(), 3);
        assert_eq!(
            result.runs().first().map(TaskRunRecord::status),
            Some(TaskResultStatus::Baseline)
        );
        assert_eq!(
            result.runs().get(1).map(TaskRunRecord::status),
            Some(TaskResultStatus::Improved)
        );

        let third_run = result.runs().get(2).expect("missing third run");
        assert_eq!(third_run.status(), TaskResultStatus::Regressed);
        assert_eq!(third_run.delta(), -3);
        assert_eq!(third_run.feedback().next_action(), "revise prompt");
        assert_eq!(third_run.feedback().findings(), ["coverage ok"]);
    }

    #[test]
    fn marks_perfect_when_threshold_is_reached() {
        let baseline = score(0, QualityTier::CompileFailed, 0, 0, 0);
        let perfect = score(50, QualityTier::Passed, 5, 5, 5);
        let mut result = TaskResult::new(baseline, perfect);

        result.record_run(1, 1, perfect, feedback("open PR"));

        assert_eq!(result.status(), TaskResultStatus::Perfect);
        assert_eq!(
            result.runs().first().map(TaskRunRecord::status),
            Some(TaskResultStatus::Perfect)
        );
    }

    #[test]
    fn renders_task_result_json() {
        let baseline = score(0, QualityTier::CompileFailed, 0, 0, 0);
        let perfect = score(1, QualityTier::Passed, 1, 0, 0);
        let mut result = TaskResult::new(baseline, perfect);

        result.record_run(1, 1, perfect, feedback("done"));

        let json = result.to_json();

        assert!(json.contains("\"status\": \"perfect\""));
        assert!(json.contains("\"quality_tier\": \"passed\""));
        assert!(json.contains("\"eval_path\": \"runs/1/samples/1/eval.json\""));
        assert!(json.contains("\"selected_sample\": 1"));
    }

    #[test]
    #[expect(
        clippy::expect_used,
        reason = "test fixture asserts parsed run existence"
    )]
    fn parses_task_result_snapshot_from_rendered_json() -> Result<(), Box<dyn std::error::Error>> {
        let baseline = score(0, QualityTier::CompileFailed, 0, 0, 0);
        let perfect = score(100, QualityTier::Passed, 10, 10, 10);
        let mut result = TaskResult::new(baseline, perfect);

        result.record_run(
            1,
            2,
            score(10, QualityTier::TestFailed, 1, 0, 0),
            feedback("revise"),
        );
        result.record_run(
            2,
            1,
            score(100, QualityTier::Passed, 10, 10, 10),
            feedback("ready\nwith \"quoted\" data"),
        );

        let snapshot = TaskResultSnapshot::parse(&result.to_json())?;

        assert_eq!(snapshot.status(), "perfect");
        assert_eq!(snapshot.max_run_index(), Some(2));
        assert_eq!(snapshot.runs().len(), 2);
        let first_run = snapshot.runs().first().expect("missing first run");
        let second_run = snapshot.runs().get(1).expect("missing second run");

        assert_eq!(first_run.run_index(), 1);
        assert_eq!(first_run.selected_sample(), 2);
        assert_eq!(first_run.status(), "improved");
        assert_eq!(second_run.eval_path(), "runs/1/samples/1/eval.json");
        assert_eq!(second_run.next_action(), "ready\nwith \"quoted\" data");
        assert!(snapshot.has_selected_sample_eval(2, 1, "runs/1/samples/1/eval.json"));
        assert!(!snapshot.has_selected_sample_eval(2, 2, "runs/1/samples/2/eval.json"));

        Ok(())
    }

    #[test]
    #[expect(
        clippy::expect_used,
        reason = "test fixture asserts parsed run existence"
    )]
    fn hydrates_task_result_from_rendered_json() -> Result<(), Box<dyn std::error::Error>> {
        let baseline = score(0, QualityTier::CompileFailed, 0, 0, 0);
        let perfect = score(100, QualityTier::Passed, 10, 10, 10);
        let mut result = TaskResult::new(baseline, perfect);
        result.record_run(
            1,
            2,
            score(35, QualityTier::PartialSuccess, 4, 3, 2),
            feedback("continue"),
        );
        result.record_run(
            2,
            1,
            score(90, QualityTier::Passed, 9, 9, 9),
            feedback("resume"),
        );

        let mut hydrated = TaskResult::from_json(&result.to_json())?;
        hydrated.record_run(
            3,
            1,
            score(80, QualityTier::PartialSuccess, 8, 8, 8),
            feedback("revise"),
        );

        assert_eq!(hydrated.baseline_score(), baseline);
        assert_eq!(hydrated.current_score().total().as_i64(), 80);
        assert_eq!(hydrated.best_score().total().as_i64(), 90);
        assert_eq!(hydrated.perfect_score(), perfect);
        assert_eq!(hydrated.runs().len(), 3);
        let third = hydrated.runs().get(2).expect("missing resumed run");
        assert_eq!(third.previous_score().total().as_i64(), 90);
        assert_eq!(third.selected_score().total().as_i64(), 80);
        assert_eq!(third.status(), TaskResultStatus::Regressed);

        Ok(())
    }

    #[test]
    fn rejects_malformed_hydrated_task_results() {
        let baseline = score(0, QualityTier::CompileFailed, 0, 0, 0);
        let perfect = score(100, QualityTier::Passed, 10, 10, 10);
        let mut result = TaskResult::new(baseline, perfect);
        result.record_run(
            1,
            1,
            score(40, QualityTier::PartialSuccess, 4, 4, 4),
            VerifierFeedback::new(
                "runs/1/samples/1/eval.json".to_owned(),
                "summary".to_owned(),
                "none".to_owned(),
            )
            .with_findings(vec!["one".to_owned(), "two".to_owned()])
            .with_benchmark_summary("bench".to_owned())
            .with_next_action("continue".to_owned()),
        );
        let json = result.to_json();
        let invalid_status =
            json.replacen("  \"status\": \"improved\"", "  \"status\": \"mystery\"", 1);
        let invalid_tier = json.replacen(
            "\"quality_tier\": \"compile_failed\"",
            "\"quality_tier\": \"mystery\"",
            1,
        );
        let invalid_integer = json.replacen("\"total\": 0", "\"total\": nope", 1);
        let missing_feedback = json.replacen("\"feedback\"", "\"feedback_missing\"", 1);

        assert!(matches!(
            TaskResult::from_json(&invalid_status),
            Err(TaskResultParseError::InvalidStatus(_))
        ));
        assert!(matches!(
            TaskResult::from_json(&invalid_tier),
            Err(TaskResultParseError::InvalidQualityTier(_))
        ));
        assert!(matches!(
            TaskResult::from_json(&invalid_integer),
            Err(TaskResultParseError::InvalidInteger(_))
        ));
        assert!(matches!(
            TaskResult::from_json(&missing_feedback),
            Err(TaskResultParseError::MissingField(_))
        ));
    }

    #[test]
    fn rejects_malformed_task_result_snapshot() {
        assert!(matches!(
            TaskResultSnapshot::parse("{}"),
            Err(TaskResultParseError::MissingField(_))
        ));
        assert!(matches!(
            TaskResultSnapshot::parse(
                "{\n  \"status\": \"baseline\",\n  \"runs\": [\n    {\n      \"run_index\": 1,\n      \"status\": \"baseline\",\n      \"feedback\": {\n        \"eval_path\": \"eval.json\",\n        \"next_action\": \"retry\"\n      }\n    }\n  ]\n}\n"
            ),
            Err(TaskResultParseError::MissingField(_))
        ));
        assert!(matches!(
            TaskResultSnapshot::parse(
                "{\n  \"status\": \"baseline\",\n  \"runs\": [\n    {\n      \"run_index\": nope,\n      \"selected_sample\": 1,\n      \"status\": \"baseline\",\n      \"feedback\": {\n        \"eval_path\": \"eval.json\",\n        \"next_action\": \"retry\"\n      }\n    }\n  ]\n}\n"
            ),
            Err(TaskResultParseError::InvalidInteger(_))
        ));
        assert_eq!(
            TaskResultParseError::MissingField("\"status\"").to_string(),
            "missing task result field \"status\""
        );
        assert_eq!(
            TaskResultParseError::InvalidInteger("\"run_index\"").to_string(),
            "invalid task result integer \"run_index\""
        );
        assert_eq!(
            TaskResultParseError::InvalidQualityTier("mystery".to_owned()).to_string(),
            "invalid quality tier mystery"
        );
        assert_eq!(
            TaskResultParseError::InvalidStatus("mystery".to_owned()).to_string(),
            "invalid task result status mystery"
        );
        assert!(matches!(
            TaskResult::from_json(
                "{\n  \"baseline_score\": {},\n  \"current_score\": {},\n  \"best_score\": {},\n  \"perfect_score\": {},\n  \"status\": \"mystery\",\n  \"runs\": [\n  ]\n}\n"
            ),
            Err(TaskResultParseError::MissingField(_) | TaskResultParseError::InvalidStatus(_))
        ));
    }

    #[test]
    #[expect(
        clippy::expect_used,
        reason = "test fixture asserts parsed run existence"
    )]
    fn parses_empty_and_escaped_task_result_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let snapshot =
            TaskResultSnapshot::parse("{\n  \"status\": \"baseline\",\n  \"runs\": [\n  ]\n}\n")?;
        assert_eq!(snapshot.status(), "baseline");
        assert_eq!(snapshot.max_run_index(), None);

        let snapshot = TaskResultSnapshot::parse(
            "{\n  \"status\": \"baseline\",\n  \"runs\": [\n    {\n      \"run_index\": 1,\n      \"selected_sample\": 1,\n      \"status\": \"baseline\",\n      \"feedback\": {\n        \"eval_path\": \"runs/1/samples/1/eval.json\",\n        \"next_action\": \"line\\n tab\\t quote\\\" slash\\\\ unicode\\u0008\"\n      }\n    }\n  ]\n}\n",
        )
        .expect("parse escaped snapshot");
        let first = snapshot.runs().first().expect("missing run");
        assert_eq!(first.run_index(), 1);
        assert!(first.next_action().contains("unicode\u{0008}"));

        Ok(())
    }

    #[test]
    fn covers_json_string_parser_edges() {
        assert!(super::parse_json_string("not-json").is_none());
        assert!(super::parse_json_string(r#""bad\xescape""#).is_none());
        assert_eq!(
            super::parse_json_string(r#""carriage\rreturn""#).as_deref(),
            Some("carriage\rreturn")
        );
    }
}
