//! Runtime checkpoint ledger helpers.

use std::{
    fs,
    io::{self, Write},
};

use crate::{AppError, TaskLayout};

const LEDGER_OPEN_ACTION: &str = "open task ledger";
const LEDGER_READ_ACTION: &str = "read task ledger";
const LEDGER_STATE_FIELD: &str = "state";
const LEDGER_WRITE_ACTION: &str = "write task ledger";
const JSON_BACKSLASH: char = '\\';
const JSON_NEWLINE: char = '\n';
const JSON_QUOTE: char = '"';
const JSON_RETURN: char = '\r';
const JSON_TAB: char = '\t';
const STATE_VALUE_MAX_CHARS: usize = 120;

/// Runtime checkpoint ledger operations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeCheckpoint;

impl RuntimeCheckpoint {
    /// Appends a durable task-state transition to `ledger.jsonl`.
    ///
    /// # Errors
    ///
    /// Returns an error when the ledger cannot be opened or written.
    pub fn append_ledger(task: &TaskLayout, state: &str, summary: &str) -> Result<(), AppError> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(task.ledger())
            .map_err(|source| AppError::io(LEDGER_OPEN_ACTION, source))?;
        writeln!(
            file,
            "{{\"task\":\"{}\",\"state\":\"{}\",\"summary\":\"{}\"}}",
            task.task_id().as_str(),
            Self::escape_json_fragment(state),
            Self::escape_json_fragment(summary)
        )
        .map_err(|source| AppError::io(LEDGER_WRITE_ACTION, source))
    }

    /// Returns the last durable task state from `ledger.jsonl`.
    ///
    /// # Errors
    ///
    /// Returns an error when an existing ledger cannot be read.
    pub fn last_ledger_state(task: &TaskLayout) -> Result<Option<String>, AppError> {
        match fs::read_to_string(task.ledger()) {
            Ok(content) => Ok(content
                .lines()
                .rev()
                .find_map(|line| json_string_field(line, LEDGER_STATE_FIELD))),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(AppError::io(LEDGER_READ_ACTION, source)),
        }
    }

    /// Returns a bounded state label suitable for checkpoint tags.
    #[must_use]
    pub fn sanitize_state_value(value: &str) -> String {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_")
            .chars()
            .take(STATE_VALUE_MAX_CHARS)
            .collect()
    }

    /// Escapes a string fragment for ledger JSONL output.
    #[must_use]
    pub fn escape_json_fragment(value: &str) -> String {
        let mut escaped = String::new();
        for character in value.chars() {
            match character {
                JSON_QUOTE => escaped.push_str("\\\""),
                JSON_BACKSLASH => escaped.push_str("\\\\"),
                JSON_NEWLINE => escaped.push_str("\\n"),
                JSON_RETURN => escaped.push_str("\\r"),
                JSON_TAB => escaped.push_str("\\t"),
                other => escaped.push(other),
            }
        }
        escaped
    }
}

fn json_string_field(line: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\":\"");
    let value = line.split_once(&needle)?.1;
    let mut parsed = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            parsed.push(ch);
            escaped = false;
        } else if ch == JSON_BACKSLASH {
            escaped = true;
        } else if ch == JSON_QUOTE {
            return Some(parsed);
        } else {
            parsed.push(ch);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{CyanosHome, ProjectId, RuntimeCheckpoint, TaskId};

    #[test]
    fn appends_and_reads_runtime_ledger_state() -> Result<(), Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("cyanos-runtime-checkpoint-{}", std::process::id()));
        let task = CyanosHome::new(root.join("home"))
            .project(ProjectId::new("owner/repo")?)
            .task(TaskId::new("123".to_owned())?);
        fs::create_dir_all(task.root())?;

        RuntimeCheckpoint::append_ledger(&task, "brief_frozen", "frozen")?;
        RuntimeCheckpoint::append_ledger(&task, "pr_feedback", "quoted \"summary\"")?;

        assert_eq!(
            RuntimeCheckpoint::last_ledger_state(&task)?.as_deref(),
            Some("pr_feedback")
        );
        assert_eq!(
            RuntimeCheckpoint::escape_json_fragment("\\\n\r\t\""),
            "\\\\\\n\\r\\t\\\""
        );
        assert_eq!(
            RuntimeCheckpoint::sanitize_state_value("needs reviewer attention"),
            "needs_reviewer_attention"
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn reports_missing_ledger_as_empty_state() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "cyanos-runtime-checkpoint-missing-{}",
            std::process::id()
        ));
        let task = CyanosHome::new(root.join("home"))
            .project(ProjectId::new("owner/repo")?)
            .task(TaskId::new("123".to_owned())?);
        fs::create_dir_all(task.root())?;

        assert!(RuntimeCheckpoint::last_ledger_state(&task)?.is_none());
        assert_eq!(
            RuntimeCheckpoint::sanitize_state_value("  lots   of   space  "),
            "lots_of_space"
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture paths must be created")]
    #[expect(
        clippy::assertions_on_result_states,
        reason = "test asserts unreadable ledger error"
    )]
    fn handles_escaped_corrupt_and_unreadable_ledger_state() {
        assert_eq!(
            super::json_string_field(r#"{"state":"pr\\feedback"}"#, "state").as_deref(),
            Some(r"pr\feedback")
        );
        assert!(super::json_string_field(r#"{"state":"unterminated"#, "state").is_none());

        let root = std::env::temp_dir().join(format!(
            "cyanos-runtime-checkpoint-unreadable-{}",
            std::process::id()
        ));
        let task = CyanosHome::new(root.join("home"))
            .project(ProjectId::new("owner/repo").expect("valid project id"))
            .task(TaskId::new("123".to_owned()).expect("valid task id"));
        fs::create_dir_all(task.root()).expect("create task root");
        fs::create_dir_all(task.ledger()).expect("create unreadable ledger directory");

        assert!(RuntimeCheckpoint::last_ledger_state(&task).is_err());

        fs::remove_dir_all(root).expect("remove runtime root");
    }
}
