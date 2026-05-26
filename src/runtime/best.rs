//! Durable global-best metadata for a Cyanos task.

use std::{
    fmt, fs,
    io::{self, Write},
    path::Path,
};

use crate::{QualityTier, SampleScore, ScoreBreakdown, json};

const BEST_FIELD_BENCHMARK: &str = "benchmark";
const BEST_FIELD_CYANOS_COMMIT: &str = "cyanos_commit";
const BEST_FIELD_EVAL_PATH: &str = "eval_path";
const BEST_FIELD_JUDGE: &str = "judge";
const BEST_FIELD_PATCH_PATH: &str = "patch_path";
const BEST_FIELD_PR_BRANCH_COMMIT: &str = "pr_branch_commit";
const BEST_FIELD_QUALITY_TIER: &str = "quality_tier";
const BEST_FIELD_SELECTED_RUN: &str = "selected_run";
const BEST_FIELD_SELECTED_SAMPLE: &str = "selected_sample";
const BEST_FIELD_TARGET_COMMIT: &str = "target_commit";
const BEST_FIELD_TASK_ID: &str = "task_id";
const BEST_FIELD_TESTS: &str = "tests";
const BEST_FIELD_TOTAL: &str = "total";
const JSON_ESCAPE: char = '\\';
const JSON_QUOTE: char = '"';
const JSON_UNICODE_ESCAPE: char = 'u';
const UNICODE_ESCAPE_DIGITS: usize = 4;
const UNICODE_ESCAPE_RADIX: u32 = 16;

/// Durable metadata tying the promoted global best to its sample evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BestRecord {
    task_id: String,
    selected_run: usize,
    selected_sample: usize,
    score: ScoreBreakdown,
    eval_path: String,
    patch_path: String,
    target_commit: String,
    cyanos_commit: String,
    pr_branch_commit: String,
}

impl BestRecord {
    /// Creates a durable global-best record.
    #[expect(
        clippy::too_many_arguments,
        reason = "best metadata records all PRD-required source and evidence identities"
    )]
    #[must_use]
    pub fn new(
        task_id: impl Into<String>,
        selected_run: usize,
        selected_sample: usize,
        score: ScoreBreakdown,
        eval_path: impl Into<String>,
        patch_path: impl Into<String>,
        target_commit: impl Into<String>,
        cyanos_commit: impl Into<String>,
        pr_branch_commit: impl Into<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            selected_run,
            selected_sample,
            score,
            eval_path: eval_path.into(),
            patch_path: patch_path.into(),
            target_commit: target_commit.into(),
            cyanos_commit: cyanos_commit.into(),
            pr_branch_commit: pr_branch_commit.into(),
        }
    }

    /// Returns the selected outer run index.
    #[must_use]
    pub const fn selected_run(&self) -> usize {
        self.selected_run
    }

    /// Returns the selected sample id.
    #[must_use]
    pub const fn selected_sample(&self) -> usize {
        self.selected_sample
    }

    /// Returns the task id this best record belongs to.
    #[must_use]
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Returns the selected sample evaluation path label.
    #[must_use]
    pub fn eval_path(&self) -> &str {
        &self.eval_path
    }

    /// Returns the promoted target commit.
    #[must_use]
    pub fn target_commit(&self) -> &str {
        &self.target_commit
    }

    /// Returns the Cyanos evolution-branch commit.
    #[must_use]
    pub fn cyanos_commit(&self) -> &str {
        &self.cyanos_commit
    }

    /// Returns the PR branch commit.
    #[must_use]
    pub fn pr_branch_commit(&self) -> &str {
        &self.pr_branch_commit
    }

    /// Parses stable `best.json` metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when required metadata fields are missing or invalid.
    pub fn from_json(content: &str) -> Result<Self, BestRecordParseError> {
        let task_id = required_string(content, BEST_FIELD_TASK_ID)?;
        let selected_run = required_usize(content, BEST_FIELD_SELECTED_RUN)?;
        let selected_sample = required_usize(content, BEST_FIELD_SELECTED_SAMPLE)?;
        let total = required_i64(content, BEST_FIELD_TOTAL)?;
        let quality_tier = required_quality_tier(content, BEST_FIELD_QUALITY_TIER)?;
        let tests = required_i64(content, BEST_FIELD_TESTS)?;
        let benchmark = required_i64(content, BEST_FIELD_BENCHMARK)?;
        let judge = required_i64(content, BEST_FIELD_JUDGE)?;
        let eval_path = required_string(content, BEST_FIELD_EVAL_PATH)?;
        let patch_path = required_string(content, BEST_FIELD_PATCH_PATH)?;
        let target_commit = required_string(content, BEST_FIELD_TARGET_COMMIT)?;
        let cyanos_commit = required_string(content, BEST_FIELD_CYANOS_COMMIT)?;
        let pr_branch_commit = required_string(content, BEST_FIELD_PR_BRANCH_COMMIT)?;

        Ok(Self::new(
            task_id,
            selected_run,
            selected_sample,
            ScoreBreakdown::new(
                SampleScore::new(total),
                quality_tier,
                SampleScore::new(tests),
                SampleScore::new(benchmark),
                SampleScore::new(judge),
            ),
            eval_path,
            patch_path,
            target_commit,
            cyanos_commit,
            pr_branch_commit,
        ))
    }

    /// Renders the record as stable JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"task_id\": {},\n",
                "  \"selected_run\": {},\n",
                "  \"selected_sample\": {},\n",
                "  \"score\": {},\n",
                "  \"eval_path\": {},\n",
                "  \"patch_path\": {},\n",
                "  \"target_commit\": {},\n",
                "  \"cyanos_commit\": {},\n",
                "  \"pr_branch_commit\": {}\n",
                "}}\n"
            ),
            json::string(&self.task_id),
            self.selected_run,
            self.selected_sample,
            self.score.render_json("  "),
            json::string(&self.eval_path),
            json::string(&self.patch_path),
            json::string(&self.target_commit),
            json::string(&self.cyanos_commit),
            json::string(&self.pr_branch_commit),
        )
    }
}

/// Error returned when parsing durable `best.json` metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BestRecordParseError {
    message: String,
}

impl BestRecordParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for BestRecordParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BestRecordParseError {}

/// Atomically writes `best.json`.
///
/// # Errors
///
/// Returns an error if the parent directory or file cannot be written or
/// renamed into place.
pub fn write_best_record(path: &Path, record: &BestRecord) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "best path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(record.to_json().as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)
}

/// Reads and parses `best.json`.
///
/// # Errors
///
/// Returns an error when the file cannot be read or parsed.
pub fn read_best_record(path: &Path) -> io::Result<BestRecord> {
    let content = fs::read_to_string(path)?;
    BestRecord::from_json(&content)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))
}

fn required_string(content: &str, field: &'static str) -> Result<String, BestRecordParseError> {
    content
        .lines()
        .find_map(|line| json_string_field(line, field))
        .ok_or_else(|| BestRecordParseError::new(format!("best.json missing string field {field}")))
}

fn required_usize(content: &str, field: &'static str) -> Result<usize, BestRecordParseError> {
    let Some(value) = content
        .lines()
        .find_map(|line| json_field_value(line, field))
    else {
        return Err(BestRecordParseError::new(format!(
            "best.json missing integer field {field}"
        )));
    };
    value
        .trim()
        .trim_end_matches(',')
        .parse::<usize>()
        .map_err(|source| {
            BestRecordParseError::new(format!("best.json invalid integer field {field}: {source}"))
        })
}

fn required_i64(content: &str, field: &'static str) -> Result<i64, BestRecordParseError> {
    let Some(value) = content
        .lines()
        .find_map(|line| json_field_value(line, field))
    else {
        return Err(BestRecordParseError::new(format!(
            "best.json missing score field {field}"
        )));
    };
    value
        .trim()
        .trim_end_matches(',')
        .parse::<i64>()
        .map_err(|source| {
            BestRecordParseError::new(format!("best.json invalid score field {field}: {source}"))
        })
}

fn required_quality_tier(
    content: &str,
    field: &'static str,
) -> Result<QualityTier, BestRecordParseError> {
    match required_string(content, field)?.as_str() {
        "compile_failed" => Ok(QualityTier::CompileFailed),
        "test_failed" => Ok(QualityTier::TestFailed),
        "partial_success" => Ok(QualityTier::PartialSuccess),
        "passed" => Ok(QualityTier::Passed),
        value => Err(BestRecordParseError::new(format!(
            "best.json invalid quality tier {value}"
        ))),
    }
}

fn json_string_field(line: &str, field: &'static str) -> Option<String> {
    json_field_value(line, field).and_then(parse_json_string)
}

fn json_field_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let trimmed = line.trim();
    let field = format!("\"{field}\"");
    let rest = trimmed.strip_prefix(&field)?;
    rest.trim_start().strip_prefix(':').map(str::trim_start)
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
    while let Some(character) = chars.next() {
        match character {
            JSON_QUOTE => return Some(parsed),
            JSON_ESCAPE => parsed.push(parse_json_escape(chars)?),
            value => parsed.push(value),
        }
    }
    None
}

fn parse_json_escape<I>(chars: &mut I) -> Option<char>
where
    I: Iterator<Item = char>,
{
    match chars.next()? {
        JSON_QUOTE => Some(JSON_QUOTE),
        JSON_ESCAPE => Some(JSON_ESCAPE),
        '/' => Some('/'),
        'b' => Some('\u{0008}'),
        'f' => Some('\u{000C}'),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_writes_best_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("cyanos-best-record-{}", std::process::id()));
        let path = root.join("best.json");
        let record = BestRecord::new(
            "123",
            2,
            3,
            ScoreBreakdown::new(
                SampleScore::new(9000),
                QualityTier::Passed,
                SampleScore::new(10_000),
                SampleScore::new(9000),
                SampleScore::new(10_000),
            ),
            "runs/2/samples/3/eval.json",
            "runs/2/samples/3/patch.diff",
            "abc",
            "def",
            "ghi",
        );
        assert_eq!(record.selected_run(), 2);
        assert_eq!(record.selected_sample(), 3);

        write_best_record(&path, &record)?;
        let json = fs::read_to_string(&path)?;
        assert!(json.contains("\"selected_run\": 2"));
        assert!(json.contains("\"selected_sample\": 3"));
        assert!(json.contains("\"total\": 9000"));
        assert!(json.contains("\"eval_path\": \"runs/2/samples/3/eval.json\""));
        assert!(json.contains("\"cyanos_commit\": \"def\""));
        let parsed = read_best_record(&path)?;
        assert_eq!(parsed.selected_run(), 2);
        assert_eq!(parsed.selected_sample(), 3);
        assert_eq!(parsed.target_commit(), "abc");
        assert_eq!(parsed.cyanos_commit(), "def");
        assert_eq!(parsed.pr_branch_commit(), "ghi");

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn rejects_best_path_without_parent() -> Result<(), Box<dyn std::error::Error>> {
        let record = BestRecord::new(
            "123",
            1,
            1,
            ScoreBreakdown::new(
                SampleScore::new(1),
                QualityTier::Passed,
                SampleScore::new(1),
                SampleScore::new(1),
                SampleScore::new(1),
            ),
            "eval.json",
            "patch.diff",
            "abc",
            "abc",
            "abc",
        );
        let error = match write_best_record(Path::new(""), &record) {
            Ok(()) => return Err(io::Error::other("empty path should fail").into()),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        Ok(())
    }

    #[test]
    fn rejects_malformed_best_metadata() -> Result<(), Box<dyn std::error::Error>> {
        if BestRecord::from_json("{}").is_ok() {
            return Err(io::Error::other("missing best fields should fail").into());
        }
        let missing_integer = BestRecord::from_json("{\n  \"task_id\": \"123\"\n}\n");
        assert!(
            parse_error_message(missing_integer).contains("missing integer field selected_run")
        );
        if BestRecord::from_json(
            "{\n  \"task_id\": \"123\",\n  \"selected_run\": nope,\n  \"selected_sample\": 1\n}\n",
        )
        .is_ok()
        {
            return Err(io::Error::other("invalid best integer should fail").into());
        }
        let missing_score = BestRecord::from_json(
            "{\n  \"task_id\": \"123\",\n  \"selected_run\": 1,\n  \"selected_sample\": 1\n}\n",
        );
        assert!(parse_error_message(missing_score).contains("missing score field total"));
        let invalid_score = BestRecord::from_json(
            "{\n  \"task_id\": \"123\",\n  \"selected_run\": 1,\n  \"selected_sample\": 1,\n  \"total\": nope\n}\n",
        );
        assert!(parse_error_message(invalid_score).contains("invalid score field total"));
        let invalid_tier = BestRecord::from_json(
            "{\n  \"task_id\": \"123\",\n  \"selected_run\": 1,\n  \"selected_sample\": 1,\n  \"total\": 1,\n  \"quality_tier\": \"unknown\"\n}\n",
        );
        assert!(parse_error_message(invalid_tier).contains("invalid quality tier unknown"));
        assert!(parse_json_string("not-json").is_none());
        assert!(parse_json_string("\"unterminated").is_none());
        assert!(parse_json_string("\"dangling\\").is_none());
        assert!(parse_json_string("\"bad\\xescape\"").is_none());
        assert!(parse_json_string("\"bad\\u00xz\"").is_none());
        assert!(parse_json_string("\"bad\\u00\"").is_none());
        assert_eq!(
            parse_json_string("\"back\\bspace\"").as_deref(),
            Some("back\u{0008}space")
        );
        Ok(())
    }

    #[test]
    fn parses_best_metadata_tiers_and_escapes() -> Result<(), Box<dyn std::error::Error>> {
        let target_commit = "quote\" slash/ backslash\\ unicode\u{0008} f\u{000c} n\n r\r t\t";
        for (tier, expected_tier) in [
            ("compile_failed", QualityTier::CompileFailed),
            ("test_failed", QualityTier::TestFailed),
            ("partial_success", QualityTier::PartialSuccess),
            ("passed", QualityTier::Passed),
        ] {
            let json = format!(
                r#"{{
  "task_id": "123",
  "selected_run": 1,
  "selected_sample": 2,
  "total": 77,
  "quality_tier": "{tier}",
  "tests": 10,
  "benchmark": 20,
  "judge": 30,
  "eval_path": "runs\/1\/samples\/2\/eval.json",
  "patch_path": "runs\\1\\samples\\2\\patch.diff",
  "target_commit": "quote\" slash\/ backslash\\ unicode\u0008 f\f n\n r\r t\t",
  "cyanos_commit": "cyanos",
  "pr_branch_commit": "pr"
}}
"#
            );

            let record = BestRecord::from_json(&json)?;

            assert_eq!(record.target_commit(), target_commit);
            assert_eq!(
                record.score,
                ScoreBreakdown::new(
                    SampleScore::new(77),
                    expected_tier,
                    SampleScore::new(10),
                    SampleScore::new(20),
                    SampleScore::new(30),
                )
            );
        }
        Ok(())
    }

    fn parse_error_message(result: Result<BestRecord, BestRecordParseError>) -> String {
        match result {
            Ok(_) => "best metadata unexpectedly parsed".to_owned(),
            Err(error) => error.to_string(),
        }
    }
}
