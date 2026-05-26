use serde_json::Value;
use std::{
    env,
    ffi::OsStr,
    fmt::{self, Display},
    fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

type Result<T> = std::result::Result<T, XtaskError>;

const REPORT_MARKER: &str = "<!-- reforge-ci-report -->";
const REQUIRED_PR_SECTIONS: [&str; 8] = [
    "## 1. Requirement",
    "## 2. Implementation",
    "## 3. Architecture and Functional Impact",
    "## 4. Test and Verification Method",
    "## 5. Test Results",
    "### Functional Tests",
    "### Benchmark Tests",
    "### End-to-End Tests",
];
const TEMPLATE_PLACEHOLDERS: [&str; 6] = [
    "State the concrete user or product requirement.",
    "Describe how the change is implemented.",
    "Describe the impact on existing architecture and behavior.",
    "List the commands, scenarios, and review methods used.",
    "Include command, environment, baseline, new result, and interpretation.",
    "Include scenario, command or workflow, and result.",
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("::error::{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [command] if command == "help" => {
            print_help();
            Ok(())
        }
        [command] if command == "pr-policy" => validate_pr_policy(),
        [command] if command == "ci-report" => comment_ci_report(),
        [command, target] if command == "lint" && target == "architecture" => lint_architecture(),
        [command, target] if command == "lint" && target == "docs-language" => lint_docs_language(),
        [command, target] if command == "lint" && target == "all" => {
            lint_architecture()?;
            lint_docs_language()
        }
        [] => Err(err(
            "missing xtask command; run `cargo run -p xtask -- help`",
        )),
        _ => Err(err(format!("unknown xtask command: {}", args.join(" ")))),
    }
}

fn print_help() {
    println!(
        "Usage:\n  cargo run -p xtask -- lint architecture\n  cargo run -p xtask -- lint docs-language\n  cargo run -p xtask -- pr-policy\n  cargo run -p xtask -- ci-report"
    );
}

fn lint_architecture() -> Result<()> {
    let files = rust_files(Path::new("src"))?;
    let mut violations = Vec::new();
    let concrete_agent_patterns = [
        "Claude",
        "Codex",
        "Gemini",
        "adapters::",
        "Agent::Claude",
        "Agent::Codex",
        "Agent::Gemini",
    ];

    for file in files.iter().filter(|path| !path.starts_with("src/agent")) {
        collect_pattern_violations(file, &concrete_agent_patterns, &mut violations)?;
    }

    let abstraction_leak_patterns = [
        "ModelSelection",
        "AgentRequest",
        "CommandSpec",
        "AdapterRegistry",
        "--agent",
        "--model",
    ];
    for file in [Path::new("src/eval.rs"), Path::new("src/loop.rs")] {
        collect_pattern_violations(file, &abstraction_leak_patterns, &mut violations)?;
    }

    if violations.is_empty() {
        return Ok(());
    }

    Err(err(format!(
        "architecture lint failed:\n{}",
        violations.join("\n")
    )))
}

fn rust_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files)?;
    Ok(files)
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(path).map_err(|source| io_err("read directory", path, source))? {
        let entry =
            entry.map_err(|source| err(format!("failed to read directory entry: {source}")))?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension() == Some(OsStr::new("rs")) {
            files.push(path);
        }
    }

    Ok(())
}

fn collect_pattern_violations(
    file: &Path,
    patterns: &[&str],
    violations: &mut Vec<String>,
) -> Result<()> {
    if !file.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(file).map_err(|source| io_err("read file", file, source))?;
    for (index, line) in content.lines().enumerate() {
        if patterns.iter().any(|pattern| line.contains(pattern)) {
            violations.push(format!("{}:{}: {}", file.display(), index + 1, line.trim()));
        }
    }
    Ok(())
}

fn lint_docs_language() -> Result<()> {
    let docs = git_ls_files(&["*.md"])?;
    let mut violations = Vec::new();

    for file in docs {
        let content =
            fs::read_to_string(&file).map_err(|source| io_err("read file", &file, source))?;
        for (line_index, line) in content.lines().enumerate() {
            if contains_non_english_letter(line) {
                violations.push(format!(
                    "{}:{}: {}",
                    file.display(),
                    line_index + 1,
                    line.trim()
                ));
            }
        }
    }

    if violations.is_empty() {
        return Ok(());
    }

    Err(err(format!(
        "documentation must be written in English; emoji are allowed, but non-English letters are not:\n{}",
        violations.join("\n")
    )))
}

fn git_ls_files(patterns: &[&str]) -> Result<Vec<PathBuf>> {
    let mut command = Command::new("git");
    command.arg("ls-files");
    for pattern in patterns {
        command.arg(pattern);
    }
    let output = command
        .output()
        .map_err(|source| err(format!("failed to run git ls-files: {source}")))?;

    if !output.status.success() {
        return Err(command_error("git ls-files", &output));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|source| err(format!("git ls-files returned non-UTF-8 output: {source}")))?;
    Ok(stdout.lines().map(PathBuf::from).collect())
}

fn validate_pr_policy() -> Result<()> {
    let metadata = pr_metadata()?;

    if metadata.commit_count != 1 {
        return Err(err(format!(
            "each pull request must contain exactly one commit; found {}",
            metadata.commit_count
        )));
    }

    require_english_text("Pull request title", &metadata.title)?;
    require_english_text("Pull request body", &metadata.body)?;

    if metadata.title.chars().count() < 8 {
        return Err(err(
            "pull request title is too short to describe the change",
        ));
    }

    if is_generic_title(&metadata.title) {
        return Err(err(
            "pull request title must describe the actual change, not a generic label",
        ));
    }

    for section in REQUIRED_PR_SECTIONS {
        if !metadata.body.contains(section) {
            return Err(err(format!(
                "pull request body is missing required section: {section}"
            )));
        }
    }

    for placeholder in TEMPLATE_PLACEHOLDERS {
        if metadata.body.contains(placeholder) {
            return Err(err(format!(
                "pull request body still contains template placeholder: {placeholder}"
            )));
        }
    }

    Ok(())
}

fn pr_metadata() -> Result<PrMetadata> {
    if let (Ok(title), Ok(body), Ok(commit_count)) = (
        env::var("PR_TITLE"),
        env::var("PR_BODY"),
        env::var("PR_COMMIT_COUNT"),
    ) {
        return Ok(PrMetadata {
            title,
            body,
            commit_count: commit_count
                .parse::<usize>()
                .map_err(|source| err(format!("invalid PR_COMMIT_COUNT: {source}")))?,
        });
    }

    if env::var("GITHUB_EVENT_NAME").as_deref() == Ok("pull_request") {
        let event = github_event()?;
        let pull_request = event
            .get("pull_request")
            .ok_or_else(|| err("pull_request event payload is missing pull_request"))?;
        return Ok(PrMetadata {
            title: json_string(pull_request, "title")?,
            body: json_optional_string(pull_request, "body")?.unwrap_or_default(),
            commit_count: json_u64(pull_request, "commits")? as usize,
        });
    }

    let ref_name = required_env("GITHUB_REF_NAME")?;
    let repository = required_env("GITHUB_REPOSITORY")?;
    let json = gh_output([
        "pr",
        "view",
        ref_name.as_str(),
        "--repo",
        repository.as_str(),
        "--json",
        "title,body,commits",
    ])?;
    let value = parse_json(&json)?;
    let commits = value
        .get("commits")
        .and_then(Value::as_array)
        .ok_or_else(|| err("gh pr view output is missing commits array"))?;

    Ok(PrMetadata {
        title: json_string(&value, "title")?,
        body: json_optional_string(&value, "body")?.unwrap_or_default(),
        commit_count: commits.len(),
    })
}

fn require_english_text(label: &str, value: &str) -> Result<()> {
    if contains_non_english_letter(value) {
        return Err(err(format!(
            "{label} must be written in English; emoji are allowed, but non-English letters are not"
        )));
    }
    Ok(())
}

fn contains_non_english_letter(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_alphabetic() && !character.is_ascii())
}

fn is_generic_title(title: &str) -> bool {
    matches!(
        title.to_ascii_lowercase().as_str(),
        "update" | "changes" | "misc" | "wip" | "fix" | "feature" | "refactor"
    )
}

fn comment_ci_report() -> Result<()> {
    required_env("GH_TOKEN")?;
    let repository = required_env("GITHUB_REPOSITORY")?;
    let run_id = required_env("GITHUB_RUN_ID")?;
    let server_url = required_env("GITHUB_SERVER_URL")?;
    let test_report = env::var("TEST_REPORT_PATH").unwrap_or_else(|_| "test-report.txt".to_owned());
    let coverage_report =
        env::var("COVERAGE_REPORT_PATH").unwrap_or_else(|_| "coverage-report.txt".to_owned());
    let pr_number = resolve_pr_number(&repository)?;
    let commit_sha = resolve_commit_sha(&repository, pr_number)?;
    let run_url = format!("{server_url}/{repository}/actions/runs/{run_id}");
    let comment = build_report_comment(
        &commit_sha,
        &run_id,
        &run_url,
        Path::new(&test_report),
        Path::new(&coverage_report),
    );

    upsert_pr_comment(&repository, pr_number, &comment)
}

fn resolve_pr_number(repository: &str) -> Result<u64> {
    if env::var("GITHUB_EVENT_NAME").as_deref() == Ok("pull_request") {
        let event = github_event()?;
        return json_u64(
            event
                .get("pull_request")
                .ok_or_else(|| err("pull_request event payload is missing pull_request"))?,
            "number",
        );
    }

    let ref_name = required_env("GITHUB_REF_NAME")?;
    let json = gh_output([
        "pr",
        "view",
        ref_name.as_str(),
        "--repo",
        repository,
        "--json",
        "number",
    ])?;
    json_u64(&parse_json(&json)?, "number")
}

fn resolve_commit_sha(repository: &str, pr_number: u64) -> Result<String> {
    if let Ok(json) = gh_output([
        "pr",
        "view",
        &pr_number.to_string(),
        "--repo",
        repository,
        "--json",
        "headRefOid",
    ]) {
        let value = parse_json(&json)?;
        if let Some(sha) = json_optional_string(&value, "headRefOid")? {
            return Ok(sha);
        }
    }

    if env::var("GITHUB_EVENT_NAME").as_deref() == Ok("pull_request") {
        let event = github_event()?;
        let pull_request = event
            .get("pull_request")
            .ok_or_else(|| err("pull_request event payload is missing pull_request"))?;
        let head = pull_request
            .get("head")
            .ok_or_else(|| err("pull_request event payload is missing head"))?;
        return json_string(head, "sha");
    }

    Ok(env::var("GITHUB_SHA").unwrap_or_else(|_| "unknown".to_owned()))
}

fn build_report_comment(
    commit_sha: &str,
    run_id: &str,
    run_url: &str,
    test_report: &Path,
    coverage_report: &Path,
) -> String {
    let test_report_text = read_report(test_report);
    let coverage_report_text = read_report(coverage_report);
    let test_summary = summarize_tests(&test_report_text);
    let coverage_summary = summarize_coverage(&coverage_report_text);
    let mut comment = String::new();

    push_line(&mut comment, REPORT_MARKER);
    push_line(&mut comment, "## CI Test and Coverage Report");
    push_line(&mut comment, "");
    push_line(&mut comment, &format!("- Commit: `{commit_sha}`"));
    push_line(
        &mut comment,
        &format!("- Workflow run: [{run_id}]({run_url})"),
    );
    push_line(&mut comment, &format!("- Tests: {test_summary}"));
    push_line(&mut comment, &format!("- Coverage: {coverage_summary}"));
    push_line(&mut comment, "");
    push_line(&mut comment, "### Test Results");
    push_line(&mut comment, "");
    push_str(&mut comment, &test_table(&test_report_text));
    push_line(&mut comment, "");
    push_line(&mut comment, "### Coverage");
    push_line(&mut comment, "");
    push_str(&mut comment, &coverage_summary_table(&coverage_report_text));
    push_line(&mut comment, "");
    push_line(&mut comment, "### Lowest Line Coverage");
    push_line(&mut comment, "");
    push_str(&mut comment, &lowest_coverage_table(&coverage_report_text));

    if has_test_failure(&test_report_text) || has_coverage_failure(&coverage_report_text) {
        push_line(&mut comment, "");
        push_line(&mut comment, "<details>");
        push_line(&mut comment, "<summary>Failure output</summary>");
        push_line(&mut comment, "");
        push_line(&mut comment, "```text");
        push_str(&mut comment, &report_tail(&test_report_text, 80));
        push_str(&mut comment, &report_tail(&coverage_report_text, 80));
        push_line(&mut comment, "```");
        push_line(&mut comment, "");
        push_line(&mut comment, "</details>");
    }

    push_line(&mut comment, "");
    push_line(
        &mut comment,
        "Full raw logs are available in the workflow run.",
    );
    comment
}

fn push_line(buffer: &mut String, line: &str) {
    buffer.push_str(line);
    buffer.push('\n');
}

fn push_str(buffer: &mut String, value: &str) {
    buffer.push_str(value);
    if !value.ends_with('\n') {
        buffer.push('\n');
    }
}

fn read_report(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|text| strip_ansi(&text))
        .unwrap_or_else(|_| "Report was not generated.".to_owned())
}

fn strip_ansi(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        if character == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for ansi_char in chars.by_ref() {
                if ('@'..='~').contains(&ansi_char) {
                    break;
                }
            }
        } else {
            output.push(character);
        }
    }

    output
}

fn summarize_tests(report: &str) -> String {
    let rows = test_rows(report);
    if rows.is_empty() {
        return "unavailable".to_owned();
    }

    let suites = rows.len();
    let passed: u64 = rows.iter().map(|row| row.passed).sum();
    let failed: u64 = rows.iter().map(|row| row.failed).sum();
    let ignored: u64 = rows.iter().map(|row| row.ignored).sum();
    format!("{suites} suites, {passed} passed, {failed} failed, {ignored} ignored")
}

fn test_table(report: &str) -> String {
    let rows = test_rows(report);
    let mut table = String::from("| Suite | Result | Passed | Failed | Ignored | Duration |\n");
    table.push_str("| --- | --- | ---: | ---: | ---: | ---: |\n");

    if rows.is_empty() {
        table.push_str("| _Unavailable_ | unknown | - | - | - | - |\n");
        return table;
    }

    for row in rows {
        table.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} |\n",
            escape_table_cell(&row.suite),
            row.result,
            row.passed,
            row.failed,
            row.ignored,
            row.duration
        ));
    }

    table
}

fn test_rows(report: &str) -> Vec<TestRow> {
    let mut suite = "unknown".to_owned();
    let mut rows = Vec::new();

    for line in report.lines() {
        let trimmed = line.trim_start();
        if let Some(running) = trimmed.strip_prefix("Running ") {
            if let Some((name, _)) = running.split_once(" (") {
                suite = name.to_owned();
            }
            continue;
        }

        if let Some(row) = parse_test_result_line(line, &suite) {
            rows.push(row);
            suite = "unknown".to_owned();
        }
    }

    rows
}

fn parse_test_result_line(line: &str, suite: &str) -> Option<TestRow> {
    let rest = line.strip_prefix("test result: ")?;
    let (result, counts) = rest.split_once(". ")?;
    let passed = parse_count_before(counts, " passed;")?;
    let failed = parse_count_before(counts, " failed;")?;
    let ignored = parse_count_before(counts, " ignored;")?;
    let duration = counts
        .split("finished in ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .to_owned();

    Some(TestRow {
        suite: suite.to_owned(),
        result: result.to_owned(),
        passed,
        failed,
        ignored,
        duration,
    })
}

fn parse_count_before(text: &str, marker: &str) -> Option<u64> {
    let before = text.split(marker).next()?;
    before.split_whitespace().last()?.parse().ok()
}

fn summarize_coverage(report: &str) -> String {
    match coverage_total(report) {
        Some(total) => {
            let gate = if total.line_coverage >= 95.0 {
                "passed"
            } else {
                "failed"
            };
            format!(
                "{:.2}% line coverage, {} / {} missed lines, gate {gate}",
                total.line_coverage, total.missed_lines, total.lines
            )
        }
        None => "unavailable".to_owned(),
    }
}

fn coverage_summary_table(report: &str) -> String {
    let mut table = String::from("| Metric | Value |\n| --- | ---: |\n");

    if let Some(total) = coverage_total(report) {
        table.push_str(&format!(
            "| Region coverage | {:.2}% |\n",
            total.region_coverage
        ));
        table.push_str(&format!(
            "| Function coverage | {:.2}% |\n",
            total.function_coverage
        ));
        table.push_str(&format!(
            "| Line coverage | {:.2}% |\n",
            total.line_coverage
        ));
        table.push_str(&format!(
            "| Missed lines | {} / {} |\n",
            total.missed_lines, total.lines
        ));
        table.push_str(&format!(
            "| Coverage gate | {} (threshold: 95%) |\n",
            if total.line_coverage >= 95.0 {
                "Passed"
            } else {
                "Failed"
            }
        ));
    } else {
        table.push_str("| Coverage report | Unavailable |\n");
    }

    table
}

fn lowest_coverage_table(report: &str) -> String {
    let mut rows = coverage_rows(report);
    rows.sort_by(|left, right| {
        left.line_coverage
            .partial_cmp(&right.line_coverage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(5);

    if rows.is_empty() {
        return "_Coverage rows unavailable._\n".to_owned();
    }

    let mut table = String::from("| File | Lines | Missed | Line Coverage |\n");
    table.push_str("| --- | ---: | ---: | ---: |\n");
    for row in rows {
        table.push_str(&format!(
            "| `{}` | {} | {} | {:.2}% |\n",
            escape_table_cell(&row.file),
            row.lines,
            row.missed_lines,
            row.line_coverage
        ));
    }
    table
}

fn coverage_total(report: &str) -> Option<CoverageTotal> {
    coverage_line(report, "TOTAL").map(|row| CoverageTotal {
        region_coverage: row.region_coverage,
        function_coverage: row.function_coverage,
        lines: row.lines,
        missed_lines: row.missed_lines,
        line_coverage: row.line_coverage,
    })
}

fn coverage_rows(report: &str) -> Vec<CoverageRow> {
    report
        .lines()
        .filter_map(|line| {
            let row = coverage_line(line, "")?;
            (row.file != "TOTAL").then_some(row)
        })
        .collect()
}

fn coverage_line(line: &str, required_name: &str) -> Option<CoverageRow> {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 10 {
        return None;
    }
    if !required_name.is_empty() && fields.first().copied() != Some(required_name) {
        return None;
    }
    if fields.first().copied() == Some("Filename") || fields.first().copied() == Some("-----") {
        return None;
    }

    Some(CoverageRow {
        file: fields.first()?.to_string(),
        region_coverage: parse_percent(fields.get(3)?)?,
        function_coverage: parse_percent(fields.get(6)?)?,
        lines: fields.get(7)?.parse().ok()?,
        missed_lines: fields.get(8)?.parse().ok()?,
        line_coverage: parse_percent(fields.get(9)?)?,
    })
}

fn parse_percent(value: &str) -> Option<f64> {
    value.strip_suffix('%')?.parse().ok()
}

fn has_test_failure(report: &str) -> bool {
    test_rows(report).iter().any(|row| row.failed > 0)
}

fn has_coverage_failure(report: &str) -> bool {
    coverage_total(report).is_none_or(|total| total.line_coverage < 95.0)
}

fn report_tail(report: &str, lines: usize) -> String {
    let all_lines = report.lines().collect::<Vec<_>>();
    let start = all_lines.len().saturating_sub(lines);
    all_lines[start..].join("\n")
}

fn escape_table_cell(value: &str) -> String {
    value.replace('|', "\\|")
}

fn upsert_pr_comment(repository: &str, pr_number: u64, body: &str) -> Result<()> {
    let comments_json = gh_output([
        "api",
        &format!("repos/{repository}/issues/{pr_number}/comments"),
    ])?;
    let comments = parse_json(&comments_json)?;
    let existing_id = comments.as_array().and_then(|comments| {
        comments.iter().rev().find_map(|comment| {
            let body = comment.get("body")?.as_str()?;
            body.contains(REPORT_MARKER)
                .then(|| comment.get("id")?.as_u64())
                .flatten()
        })
    });

    let request = serde_json::json!({ "body": body }).to_string();
    if let Some(comment_id) = existing_id {
        gh_with_stdin(
            [
                "api",
                "--method",
                "PATCH",
                &format!("repos/{repository}/issues/comments/{comment_id}"),
                "--input",
                "-",
            ],
            &request,
        )?;
    } else {
        gh_with_stdin(
            [
                "api",
                "--method",
                "POST",
                &format!("repos/{repository}/issues/{pr_number}/comments"),
                "--input",
                "-",
            ],
            &request,
        )?;
    }

    Ok(())
}

fn github_event() -> Result<Value> {
    let path = required_env("GITHUB_EVENT_PATH")?;
    let content =
        fs::read_to_string(&path).map_err(|source| io_err("read GitHub event", &path, source))?;
    parse_json(&content)
}

fn json_string(value: &Value, key: &str) -> Result<String> {
    json_optional_string(value, key)?
        .ok_or_else(|| err(format!("missing JSON string field: {key}")))
}

fn json_optional_string(value: &Value, key: &str) -> Result<Option<String>> {
    value
        .get(key)
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| err(format!("JSON field is not a string: {key}")))
        })
        .transpose()
}

fn json_u64(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| err(format!("missing JSON integer field: {key}")))
}

fn parse_json(input: &str) -> Result<Value> {
    serde_json::from_str(input).map_err(|source| err(format!("failed to parse JSON: {source}")))
}

fn required_env(name: &str) -> Result<String> {
    env::var(name).map_err(|source| err(format!("{name} is required: {source}")))
}

fn gh_output<const N: usize>(args: [&str; N]) -> Result<String> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .map_err(|source| err(format!("failed to run gh: {source}")))?;

    if !output.status.success() {
        return Err(command_error("gh", &output));
    }

    String::from_utf8(output.stdout)
        .map_err(|source| err(format!("gh returned non-UTF-8 output: {source}")))
}

fn gh_with_stdin<const N: usize>(args: [&str; N], stdin: &str) -> Result<()> {
    let mut child = Command::new("gh")
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|source| err(format!("failed to run gh: {source}")))?;

    if let Some(mut child_stdin) = child.stdin.take() {
        io::Write::write_all(&mut child_stdin, stdin.as_bytes())
            .map_err(|source| err(format!("failed to write gh stdin: {source}")))?;
    } else {
        return Err(err("failed to open gh stdin"));
    }

    let output = child
        .wait_with_output()
        .map_err(|source| err(format!("failed to wait for gh: {source}")))?;

    if !output.status.success() {
        return Err(command_error("gh", &output));
    }

    Ok(())
}

fn command_error(command: &str, output: &std::process::Output) -> XtaskError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    err(format!(
        "{command} failed with status {}: {}{}",
        output.status,
        stderr.trim(),
        stdout.trim()
    ))
}

fn io_err(action: &str, path: &Path, source: io::Error) -> XtaskError {
    err(format!("failed to {action} {}: {source}", path.display()))
}

fn err(message: impl Into<String>) -> XtaskError {
    XtaskError(message.into())
}

#[derive(Debug)]
struct XtaskError(String);

impl Display for XtaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for XtaskError {}

struct PrMetadata {
    title: String,
    body: String,
    commit_count: usize,
}

struct TestRow {
    suite: String,
    result: String,
    passed: u64,
    failed: u64,
    ignored: u64,
    duration: String,
}

struct CoverageRow {
    file: String,
    region_coverage: f64,
    function_coverage: f64,
    lines: u64,
    missed_lines: u64,
    line_coverage: f64,
}

struct CoverageTotal {
    region_coverage: f64,
    function_coverage: f64,
    lines: u64,
    missed_lines: u64,
    line_coverage: f64,
}

#[cfg(test)]
mod tests {
    use super::{
        build_report_comment, contains_non_english_letter, coverage_total, has_coverage_failure,
        has_test_failure, strip_ansi, summarize_tests, test_rows,
    };
    use std::path::Path;

    #[test]
    fn detects_non_english_letters_but_allows_emoji() {
        assert!(!contains_non_english_letter("English docs are required 🚀"));
        assert!(contains_non_english_letter("中文 docs are rejected"));
    }

    #[test]
    fn parses_test_summary() {
        let report = "\
     Running unittests src/lib.rs (target/debug/deps/reforge)

running 2 tests
test a ... ok
test b ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
";

        assert_eq!(
            summarize_tests(report),
            "1 suites, 2 passed, 0 failed, 0 ignored"
        );
        assert_eq!(test_rows(report).len(), 1);
        assert!(!has_test_failure(report));
    }

    #[test]
    fn parses_coverage_gate() {
        let report = "\
Filename Regions Missed Regions Cover Functions Missed Functions Executed Lines Missed Lines Cover Branches Missed Branches Cover
file.rs 10 0 100.00% 2 0 100.00% 20 1 95.00% 0 0 -
TOTAL 10 0 100.00% 2 0 100.00% 20 1 95.00% 0 0 -
";
        let total = coverage_total(report)
            .ok_or("missing coverage total")
            .unwrap_or_else(|error| {
                panic!("{error}");
            });

        assert_eq!(total.lines, 20);
        assert!(!has_coverage_failure(report));
    }

    #[test]
    fn strips_ansi_sequences() {
        assert_eq!(strip_ansi("\u{1b}[1mhello\u{1b}[0m"), "hello");
    }

    #[test]
    fn builds_readable_report_without_raw_logs_on_success() {
        let comment = build_report_comment(
            "abc123",
            "42",
            "https://example.test/run",
            Path::new("missing-test-report.txt"),
            Path::new("missing-coverage-report.txt"),
        );

        assert!(comment.contains("## CI Test and Coverage Report"));
        assert!(comment.contains("| Suite | Result | Passed | Failed | Ignored | Duration |"));
        assert!(!comment.contains("<summary>Test report</summary>"));
    }
}
