//! Agent JSON stream parser.

use super::{AgentStreamEvent, AgentStreamToolCallEvent};

const DEFAULT_TOOL_SUMMARY: &str = "tool call";
const MAX_SUMMARY_CHARS: usize = 120;
const SUMMARY_KEY: &str = "summary";
const EXEC_COMMAND_KIND: &str = "exec_command";
const COMMAND_EXECUTION_KIND: &str = "command_execution";
const QUOTE: char = '"';
const BACKSLASH: char = '\\';
const OPEN_OBJECT: char = '{';
const CLOSE_OBJECT: char = '}';
const OPEN_ARRAY: char = '[';
const CLOSE_ARRAY: char = ']';
const COLON: char = ':';

/// Extracts user-facing events from agent JSONL output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgentStreamParser;

impl AgentStreamParser {
    /// Parses one stdout line from an agent stream.
    #[must_use]
    pub fn parse_line(line: &str) -> Vec<AgentStreamEvent> {
        let mut events = Vec::new();

        for object in object_slices(line) {
            if let Some(event) = tool_event_from_object(object) {
                events.push(AgentStreamEvent::ToolCall(event));
            }
        }

        events
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AgentStreamField {
    String(String),
    Raw(String),
    Null,
}

fn object_slices(line: &str) -> Vec<&str> {
    let mut starts = Vec::new();
    let mut objects = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        if character == BACKSLASH && in_string {
            escaped = true;
            continue;
        }

        if character == QUOTE {
            in_string = !in_string;
            continue;
        }

        if in_string {
            continue;
        }

        if character == OPEN_OBJECT {
            starts.push(index);
        } else if character == CLOSE_OBJECT {
            let Some(start) = starts.pop() else {
                continue;
            };
            let end = index + character.len_utf8();
            if let Some(slice) = line.get(start..end) {
                objects.push(slice);
            }
        }
    }

    objects
}

fn tool_event_from_object(object: &str) -> Option<AgentStreamToolCallEvent> {
    if !looks_like_tool_call(object) {
        return None;
    }

    let tool_name = tool_name(object)?;
    let summary = tool_summary(object);

    Some(AgentStreamToolCallEvent::new(tool_name, summary))
}

fn looks_like_tool_call(object: &str) -> bool {
    let Some(kind) = string_field(object, "type").or_else(|| string_field(object, "event")) else {
        return false;
    };
    let normalized = kind.to_ascii_lowercase();

    normalized.contains("tool")
        || normalized.contains("function_call")
        || normalized.contains(EXEC_COMMAND_KIND)
        || normalized.contains(COMMAND_EXECUTION_KIND)
}

fn tool_name(object: &str) -> Option<String> {
    for key in ["tool_name", "tool", "name", "function", "command_name"] {
        if let Some(value) = string_field(object, key) {
            return Some(value);
        }
    }

    if string_field(object, "command").is_some() || string_field(object, "cmd").is_some() {
        return Some("shell".to_owned());
    }

    None
}

fn tool_summary(object: &str) -> String {
    for key in [
        SUMMARY_KEY,
        "description",
        "command",
        "cmd",
        "query",
        "path",
        "file_path",
        "pattern",
        "arguments",
    ] {
        if let Some(value) = string_field(object, key) {
            return truncate_summary(&value);
        }
    }

    for key in ["input", "args"] {
        if let Some(value) = field_value(object, key).and_then(|value| input_summary(&value)) {
            return value;
        }
    }

    DEFAULT_TOOL_SUMMARY.to_owned()
}

fn input_summary(value: &AgentStreamField) -> Option<String> {
    match value {
        AgentStreamField::String(value) => Some(truncate_summary(value)),
        AgentStreamField::Raw(value) => {
            for key in [
                SUMMARY_KEY,
                "description",
                "command",
                "cmd",
                "query",
                "path",
                "file_path",
                "pattern",
            ] {
                if let Some(value) = string_field(value, key) {
                    return Some(truncate_summary(&value));
                }
            }

            Some(truncate_summary(value))
        }
        AgentStreamField::Null => None,
    }
}

fn string_field(object: &str, key: &str) -> Option<String> {
    match field_value(object, key)? {
        AgentStreamField::String(value) => Some(value),
        AgentStreamField::Raw(_) | AgentStreamField::Null => None,
    }
}

fn field_value(object: &str, key: &str) -> Option<AgentStreamField> {
    let key_start = find_key(object, key)?;
    let after_key = object.get(key_start..)?;
    let colon_offset = after_key.find(COLON)?;
    let after_colon = after_key
        .get(colon_offset + COLON.len_utf8()..)?
        .trim_start();
    parse_field_value(after_colon)
}

fn find_key(object: &str, key: &str) -> Option<usize> {
    let mut cursor = 0;
    let mut depth: usize = 0;

    while let Some((offset, character)) = object.get(cursor..)?.char_indices().next() {
        let index = cursor + offset;

        if character == QUOTE {
            let (name, end) = parse_string_at(object, index)?;
            if depth == 1 && name == key {
                let after_name = object.get(end..)?.trim_start();
                if after_name.starts_with(COLON) {
                    return Some(end);
                }
            }
            cursor = end;
            continue;
        }

        if character == OPEN_OBJECT || character == OPEN_ARRAY {
            depth += 1;
        } else if character == CLOSE_OBJECT || character == CLOSE_ARRAY {
            depth = depth.saturating_sub(1);
        }

        cursor = index + character.len_utf8();
    }

    None
}

fn parse_field_value(value: &str) -> Option<AgentStreamField> {
    let first = value.chars().next()?;

    if first == QUOTE {
        let (value, _end) = parse_string_at(value, 0)?;
        return Some(AgentStreamField::String(value));
    }

    if first == OPEN_OBJECT || first == OPEN_ARRAY {
        let end = matching_container_end(value, first)?;
        return value
            .get(..end)
            .map(|value| AgentStreamField::Raw(value.to_owned()));
    }

    let end = value
        .find([',', CLOSE_OBJECT, CLOSE_ARRAY])
        .unwrap_or(value.len());
    let raw = value.get(..end)?.trim();
    if raw == "null" {
        Some(AgentStreamField::Null)
    } else {
        Some(AgentStreamField::Raw(raw.to_owned()))
    }
}

fn matching_container_end(value: &str, open: char) -> Option<usize> {
    let close = if open == OPEN_OBJECT {
        CLOSE_OBJECT
    } else {
        CLOSE_ARRAY
    };
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;

    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        if character == BACKSLASH && in_string {
            escaped = true;
            continue;
        }

        if character == QUOTE {
            in_string = !in_string;
            continue;
        }

        if in_string {
            continue;
        }

        if character == open {
            depth += 1;
        } else if character == close {
            depth -= 1;
            if depth == 0 {
                return Some(index + character.len_utf8());
            }
        }
    }

    None
}

fn parse_string_at(value: &str, start: usize) -> Option<(String, usize)> {
    if !value.get(start..)?.starts_with(QUOTE) {
        return None;
    }

    let mut output = String::new();
    let mut escaped = false;
    let content = value.get(start + QUOTE.len_utf8()..)?;

    for (offset, character) in content.char_indices() {
        let absolute = start + QUOTE.len_utf8() + offset;

        if escaped {
            push_escaped(&mut output, character);
            escaped = false;
            continue;
        }

        if character == BACKSLASH {
            escaped = true;
            continue;
        }

        if character == QUOTE {
            return Some((output, absolute + character.len_utf8()));
        }

        output.push(character);
    }

    None
}

fn push_escaped(output: &mut String, character: char) {
    match character {
        QUOTE => output.push(QUOTE),
        BACKSLASH => output.push(BACKSLASH),
        'n' => output.push('\n'),
        'r' => output.push('\r'),
        't' => output.push('\t'),
        other => output.push(other),
    }
}

fn truncate_summary(value: &str) -> String {
    let summary = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = String::new();

    for character in summary.chars().take(MAX_SUMMARY_CHARS) {
        output.push(character);
    }

    if summary.chars().count() > MAX_SUMMARY_CHARS {
        output.push_str("...");
    }

    if output.is_empty() {
        DEFAULT_TOOL_SUMMARY.to_owned()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentStreamField, AgentStreamParser, OPEN_ARRAY, OPEN_OBJECT, matching_container_end,
        parse_field_value, parse_string_at,
    };

    #[test]
    fn extracts_claude_tool_use_events() {
        let events = AgentStreamParser::parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}]}}"#,
        );

        assert_eq!(events.len(), 1);
        assert_eq!(
            events.first().map(ToString::to_string),
            Some("[Bash: cargo test]".to_owned())
        );
    }

    #[test]
    fn extracts_codex_command_events() {
        let events = AgentStreamParser::parse_line(
            r#"{"type":"exec_command_begin","command":"rg State docs/architecture.md"}"#,
        );

        assert_eq!(events.len(), 1);
        assert_eq!(
            events.first().map(ToString::to_string),
            Some("[shell: rg State docs/architecture.md]".to_owned())
        );
    }

    #[test]
    fn ignores_plain_text_and_non_tool_json() {
        assert!(AgentStreamParser::parse_line(r#"{"type":"assistant","text":"ok"}"#).is_empty());
        assert!(AgentStreamParser::parse_line("plain text").is_empty());
    }

    #[test]
    fn extracts_nested_array_events_and_argument_summaries() {
        let events = AgentStreamParser::parse_line(
            r#"[{"payload":{"items":[{"type":"function_call","name":"search","arguments":"docs architecture"}]}}]"#,
        );

        assert_eq!(
            events.first().map(ToString::to_string),
            Some("[search: docs architecture]".to_owned())
        );
    }

    #[test]
    fn summarizes_input_objects_and_empty_values() {
        let query = AgentStreamParser::parse_line(
            r#"{"type":"tool_call","tool":"Search","input":{"query":"state table"}}"#,
        );
        let empty = AgentStreamParser::parse_line(
            r#"{"type":"tool_call","tool":"Note","input":{"summary":""}}"#,
        );

        assert_eq!(
            query.first().map(ToString::to_string),
            Some("[Search: state table]".to_owned())
        );
        assert_eq!(
            empty.first().map(ToString::to_string),
            Some("[Note: tool call]".to_owned())
        );
    }

    #[test]
    fn handles_missing_names_input_shapes_and_long_summaries() {
        let missing_name = AgentStreamParser::parse_line(r#"{"type":"tool_call"}"#);
        let input_string =
            AgentStreamParser::parse_line(r#"{"type":"tool_call","name":"Note","input":"hello"}"#);
        let input_object = AgentStreamParser::parse_line(
            r#"{"type":"tool_call","name":"Note","input":{"alpha":"beta"}}"#,
        );
        let input_null =
            AgentStreamParser::parse_line(r#"{"type":"tool_call","name":"Note","input":null}"#);
        let long_summary = AgentStreamParser::parse_line(&format!(
            r#"{{"type":"tool_call","name":"Note","summary":"{}"}}"#,
            "x".repeat(130)
        ));

        assert!(missing_name.is_empty());
        assert_eq!(
            input_string.first().map(ToString::to_string),
            Some("[Note: hello]".to_owned())
        );
        assert_eq!(
            input_object.first().map(ToString::to_string),
            Some(r#"[Note: {"alpha":"beta"}]"#.to_owned())
        );
        assert_eq!(
            input_null.first().map(ToString::to_string),
            Some("[Note: tool call]".to_owned())
        );
        assert!(
            long_summary
                .first()
                .map(ToString::to_string)
                .is_some_and(|event| event.contains("..."))
        );
    }

    #[test]
    fn handles_escaped_strings_and_unmatched_noise() {
        let escaped = AgentStreamParser::parse_line(
            r#"} {"type":"tool_call","tool_name":"Write","description":"say \"hello\" } \n \r \t \\ \u"}"#,
        );

        assert_eq!(
            escaped.first().map(ToString::to_string),
            Some(r#"[Write: say "hello" } \ u]"#.to_owned())
        );
    }

    #[test]
    fn extracts_tool_names_and_summaries_from_alternate_agent_shapes() {
        let command_name = AgentStreamParser::parse_line(
            r#"{"type":"tool_call","command_name":"grep","pattern":"TODO"}"#,
        );
        let function = AgentStreamParser::parse_line(
            r#"{"event":"command_execution","function":"Glob","args":[{"path":"src/lib.rs"}]}"#,
        );
        let shell = AgentStreamParser::parse_line(r#"{"type":"tool_call","cmd":"cargo check"}"#);
        let raw_input = AgentStreamParser::parse_line(
            r#"{"type":"tool_call","name":"Decision","input":false}"#,
        );

        assert_eq!(
            command_name.first().map(ToString::to_string),
            Some("[grep: TODO]".to_owned())
        );
        assert_eq!(
            function.first().map(ToString::to_string),
            Some(r#"[Glob: [{"path":"src/lib.rs"}]]"#.to_owned())
        );
        assert_eq!(
            shell.first().map(ToString::to_string),
            Some("[shell: cargo check]".to_owned())
        );
        assert_eq!(
            raw_input.first().map(ToString::to_string),
            Some("[Decision: false]".to_owned())
        );
    }

    #[test]
    fn lower_level_json_scanners_reject_malformed_values() {
        assert_eq!(parse_string_at("name", 0), None);
        assert_eq!(parse_string_at("\"missing end", 0), None);
        assert_eq!(matching_container_end("{\"a\":1", OPEN_OBJECT), None);
        assert_eq!(parse_field_value(""), None);

        let string = parse_field_value("\"a\\nb\\rc\\t\\\\\\\"\"");
        assert!(
            matches!(string, Some(AgentStreamField::String(value)) if value == "a\nb\rc\t\\\"")
        );
        assert!(matches!(
            parse_field_value(r#"["\]"] trailing"#),
            Some(AgentStreamField::Raw(value)) if value == r#"["\]"]"#
        ));
        assert_eq!(matching_container_end(r#"["\]"]"#, OPEN_ARRAY), Some(6));
    }
}
