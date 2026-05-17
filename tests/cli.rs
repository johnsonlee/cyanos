//! CLI integration tests.

use cyanos as _;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn temp_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "cyanos-cli-{name}-{}-{}",
        std::process::id(),
        monotonic_nanos()
    ))
}

fn fake_dependency_root(name: &str) -> io::Result<PathBuf> {
    let root = temp_dir(name);
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    let bin = root.join("bin");
    fs::create_dir_all(&bin)?;

    for tool in ["gh", "cargo", "claude", "codex"] {
        write_fake_tool(&bin, tool)?;
    }

    Ok(root)
}

#[expect(
    clippy::too_many_lines,
    reason = "the integration fake keeps gh behavior in one shell fixture"
)]
fn write_fake_tool(bin: &Path, tool: &str) -> io::Result<()> {
    let path = bin.join(tool);
    let script = if tool == "gh" {
        r#"#!/bin/sh
if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
  if [ -n "$CYANOS_FAKE_REPO_VIEW_FAIL" ]; then
    exit 12
  fi
  if [ -n "$CYANOS_FAKE_REPO_PERMISSION" ]; then
    printf '%s\n' "$CYANOS_FAKE_REPO_PERMISSION"
  else
    printf '%s\n' 'WRITE'
  fi
  exit 0
fi
if [ "$1" = "repo" ] && [ "$2" = "create" ]; then
  if [ -n "$CYANOS_FAKE_REPO_CREATE_FAIL" ]; then
    exit 13
  fi
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  if [ -n "$CYANOS_FAKE_ISSUE_VIEW_FAIL" ]; then
    exit 7
  fi
  if [ -n "$CYANOS_FAKE_ISSUE_VIEW_MALFORMED" ]; then
    printf '%s\n' 'malformed issue output'
    exit 0
  fi
  printf 'https://github.com/owner/repo/issues/%s\n' "$3"
  printf '%s\n' '---CYANOS-TITLE---'
  printf '%s\n' 'Fake task'
  printf '%s\n' '---CYANOS-BODY---'
  if [ -n "$CYANOS_FAKE_ISSUE_BODY_FILE" ]; then
    cat "$CYANOS_FAKE_ISSUE_BODY_FILE"
  else
    cat <<'BODY'
## Problem

Users need a concrete change implemented by Cyanos.

## Expected Behavior

The command should run from a selected task id.

## Scope

Only the files needed for the task should change.

## Acceptance Criteria

The repository verifier must pass after the change.

## Verification

Run cargo check, cargo clippy, cargo test, and coverage.
BODY
  fi
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
  if [ -n "$CYANOS_FAKE_ISSUE_COMMENT_FAIL" ]; then
    exit 8
  fi
  if [ -n "$CYANOS_FAKE_GH_LOG" ]; then
    printf '%s\n' "$*" >> "$CYANOS_FAKE_GH_LOG"
  fi
  if [ -n "$CYANOS_FAKE_ISSUE_COMMENT_EMPTY" ]; then
    exit 0
  fi
  printf 'https://github.com/owner/repo/issues/%s#issuecomment-1\n' "$3"
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  if [ -n "$CYANOS_FAKE_PR_VIEW_FAIL" ]; then
    exit 6
  fi
  if [ "$3" = "https://github.com/owner/repo/pull/456" ]; then
    case "$*" in
      *"headRefOid,body"*)
        printf '%s\n' 'abc123'
        printf '%s\n' '## 4. Verification Method'
        printf '%s\n' '- Evaluation: runs/1/samples/1/eval.json'
        exit 0
        ;;
    esac
    if [ -n "$CYANOS_FAKE_PR_READINESS_FAIL" ]; then
      printf '%s\n' 'CHANGES_REQUESTED' 'DIRTY' 'ci FAILURE'
    else
      printf '%s\n' 'APPROVED' 'CLEAN' 'ci SUCCESS'
    fi
    exit 0
  fi
  if [ -n "$CYANOS_FAKE_PR_VIEW_URL" ]; then
    printf '%s\n' "$CYANOS_FAKE_PR_VIEW_URL"
    exit 0
  fi
  exit 1
fi
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  if [ -n "$CYANOS_FAKE_PR_LIST_AMBIGUOUS" ]; then
    printf '%s\t%s\t%s\t%s\n' 'https://github.com/owner/repo/pull/456' 'feature/installable-alpha' 'Cyanos task 123' '<!--cyanos:task=123-->'
    printf '%s\t%s\t%s\t%s\n' 'https://github.com/owner/repo/pull/789' 'feature/other' 'Fix #123' 'legacy exact task reference'
    exit 0
  fi
  if [ -n "$CYANOS_FAKE_ADOPT_PR" ]; then
    printf '%s\t%s\t%s\t%s\n' 'https://github.com/owner/repo/pull/456' 'feature/installable-alpha' 'Cyanos task 123' '<!--cyanos:task=123-->'
  fi
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  if [ -n "$CYANOS_FAKE_PR_CREATE_FAIL" ]; then
    exit 9
  fi
  printf '%s\n' 'https://github.com/owner/repo/pull/456'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "edit" ]; then
  if [ -n "$CYANOS_FAKE_PR_EDIT_FAIL" ]; then
    exit 10
  fi
  if [ -n "$CYANOS_FAKE_GH_LOG" ]; then
    printf '%s\n' "$*" >> "$CYANOS_FAKE_GH_LOG"
  fi
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "graphql" ]; then
  case "$*" in
    *"viewer"*)
      printf '%s\n' 'cyanos-bot'
      exit 0
      ;;
    *"issueOrPullRequest"*)
      if [ -n "$CYANOS_FAKE_TASK_TYPE" ]; then
        printf '%s\n' "$CYANOS_FAKE_TASK_TYPE"
      else
        printf '%s\n' 'Issue'
      fi
      exit 0
      ;;
  esac
  if [ -n "$CYANOS_FAKE_PR_REVIEW_THREADS_FAIL" ]; then
    exit 11
  fi
  case "$*" in
    *"resolveReviewThread"*)
      if [ -n "$CYANOS_FAKE_GH_LOG" ]; then
        printf '%s\n' "$*" >> "$CYANOS_FAKE_GH_LOG"
      fi
      exit 0
      ;;
  esac
  if [ -n "$CYANOS_FAKE_PR_UNRESOLVED_THREADS" ]; then
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' 'THREAD1' 'false' 'false' 'src/main.rs' '1001' 'reviewer' 'https://github.com/owner/repo/pull/456#discussion_r1' 'This drops review context. Please preserve the actual body.'
  fi
  exit 0
fi
if [ "$1" = "api" ] && [ "$2" = "-X" ] && [ "$3" = "POST" ]; then
  if [ -n "$CYANOS_FAKE_GH_LOG" ]; then
    printf '%s\n' "$*" >> "$CYANOS_FAKE_GH_LOG"
  fi
  exit 0
fi
exit 0
"#
    } else if tool == "cargo" {
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' 'cargo 1.95.0'
  exit 0
fi
case "$1" in
  check|clippy|test)
    exit 0
    ;;
  llvm-cov)
    cat <<'COVERAGE'
Filename Regions Missed Cover Functions Missed Cover Lines Missed Cover
TOTAL 1 0 100.00% 1 0 100.00% 1 0 100.00%
COVERAGE
    exit 0
    ;;
esac
exit 0
"#
    } else {
        "#!/bin/sh\nprompt=$(cat)\ncase \"$* $prompt\" in\n  *\"Cyanos code review judge\"*) printf '%s\\n' 'CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote findings=none summary=ok'; exit 0 ;;\nesac\nprintf 'fake agent output\\n'\nexit 0\n"
    };
    write_script(&path, script)
}

fn write_script(path: &Path, script: &str) -> io::Result<()> {
    let file_name = path
        .file_name()
        .map_or_else(|| "script".into(), |name| name.to_string_lossy());
    let temp = path.with_file_name(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        monotonic_nanos()
    ));
    fs::write(&temp, script)?;
    #[cfg(unix)]
    set_executable(&temp)?;
    fs::rename(temp, path)
}

fn monotonic_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
}

fn path_with_fake_dependencies(bin: &Path) -> io::Result<OsString> {
    let mut paths = vec![bin.to_path_buf()];
    if let Some(cargo) = env::var_os("CARGO") {
        let cargo = PathBuf::from(cargo);
        if let Some(parent) = cargo.parent() {
            paths.push(parent.to_path_buf());
        }
    }
    if let Some(home) = env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".cargo/bin"));
    }
    if let Some(existing) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing));
    }

    env::join_paths(paths).map_err(io::Error::other)
}

fn cyanos_command(profile_name: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cyanos"));
    if let Some(pattern) = env::var_os("LLVM_PROFILE_FILE") {
        command.env(
            "LLVM_PROFILE_FILE",
            child_coverage_profile_pattern(&pattern, profile_name),
        );
    }
    command
}

fn child_coverage_profile_pattern(pattern: &OsStr, profile_name: &str) -> OsString {
    let pattern = pattern.to_string_lossy();
    let child_pattern = if pattern.contains("%m") {
        pattern.replace("%m", &format!("{profile_name}-%p-%m"))
    } else if let Some((prefix, suffix)) = pattern.rsplit_once(".profraw") {
        format!("{prefix}-{profile_name}-%p{suffix}.profraw")
    } else {
        format!("{pattern}-{profile_name}-%p.profraw")
    };
    OsString::from(child_pattern)
}

fn completed_task_result_json(run_index: usize, next_action: &str) -> String {
    format!(
        r#"{{
  "baseline_score": {{
    "total": 0,
    "quality_tier": "compile_failed",
    "tests": 0,
    "benchmark": 0,
    "judge": 0
  }},
  "current_score": {{
    "total": 8000,
    "quality_tier": "passed",
    "tests": 8000,
    "benchmark": 8000,
    "judge": 8000
  }},
  "best_score": {{
    "total": 8000,
    "quality_tier": "passed",
    "tests": 8000,
    "benchmark": 8000,
    "judge": 8000
  }},
  "perfect_score": {{
    "total": 10000,
    "quality_tier": "passed",
    "tests": 10000,
    "benchmark": 10000,
    "judge": 10000
  }},
  "status": "improved",
  "runs": [
    {{
      "run_index": {run_index},
      "selected_sample": 1,
      "previous_score": {{
        "total": 0,
        "quality_tier": "compile_failed",
        "tests": 0,
        "benchmark": 0,
        "judge": 0
      }},
      "selected_score": {{
        "total": 8000,
        "quality_tier": "passed",
        "tests": 8000,
        "benchmark": 8000,
        "judge": 8000
      }},
      "delta": 8000,
      "status": "improved",
      "feedback": {{
        "eval_path": "runs/{run_index}/samples/1/eval.json",
        "summary": "sample passed repository verifier",
        "failure_class": "none",
        "findings": [],
        "benchmark_summary": "",
        "next_action": "{next_action}"
      }}
    }}
  ]
}}
"#
    )
}

fn setup_managed_origin(home: &Path, project_id: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(home)?;
    let project_root = home.join(".cyanos").join("projects").join(project_id);
    let origin = project_root.join("origin");
    fs::create_dir_all(&origin)?;
    let runtime_remote = home
        .join(".cyanos")
        .join("runtime-remotes")
        .join(format!("{project_id}.git"));
    fs::create_dir_all(
        runtime_remote
            .parent()
            .ok_or_else(|| io::Error::other("missing runtime remote parent"))?,
    )?;
    run_git(
        home,
        &["init", "--bare", runtime_remote.to_string_lossy().as_ref()],
    )?;
    run_git(&project_root, &["init"])?;
    run_git(&project_root, &["checkout", "-B", "main"])?;
    run_git(&project_root, &["config", "user.name", "Cyanos Test"])?;
    run_git(
        &project_root,
        &["config", "user.email", "cyanos-test@example.com"],
    )?;
    run_git(
        &project_root,
        &[
            "remote",
            "add",
            "origin",
            runtime_remote.to_string_lossy().as_ref(),
        ],
    )?;
    fs::write(
        project_root.join(".gitignore"),
        "origin/\ncyanos.lock\ntasks/*/worktree/\ntasks/*/runs/*/samples/*/worktree/\n",
    )?;
    run_git(&origin, &["init"])?;
    run_git(&origin, &["checkout", "-B", "main"])?;
    run_git(&origin, &["config", "user.name", "Cyanos Test"])?;
    run_git(
        &origin,
        &["config", "user.email", "cyanos-test@example.com"],
    )?;
    fs::write(origin.join("README.md"), "# Test repository\n")?;
    run_git(&origin, &["add", "README.md"])?;
    run_git(&origin, &["commit", "-m", "Add repository scaffold"])?;
    fs::write(
        project_root.join("config.txt"),
        format!(
            "repo=owner/repo\nruntime_repo=owner/cyanos-repo\nbase_branch=main\nsource_path={}\n",
            origin.display()
        ),
    )?;

    Ok(origin)
}

fn setup_push_remote(origin: &Path, root: &Path) -> io::Result<PathBuf> {
    let remote = root.join("remote.git");
    run_git(root, &["init", "--bare", remote.to_string_lossy().as_ref()])?;
    run_git(
        origin,
        &["remote", "add", "origin", remote.to_string_lossy().as_ref()],
    )?;
    run_git(origin, &["push", "-u", "origin", "main"])?;
    run_git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"])?;
    Ok(remote)
}

fn write_smoke_rust_target(origin: &Path, expected_answer: u8) -> io::Result<()> {
    fs::write(
        origin.join("Cargo.toml"),
        "[package]\nname = \"smoke-target\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )?;
    fs::create_dir_all(origin.join("src"))?;
    fs::write(
        origin.join("src/lib.rs"),
        format!(
            "pub fn answer() -> u8 {{ 41 }}\n\n#[cfg(test)]\nmod tests {{\n    #[test]\n    fn answer_is_42() {{\n        assert_eq!(super::answer(), {expected_answer});\n    }}\n}}\n"
        ),
    )?;
    run_git(origin, &["add", "Cargo.toml", "src/lib.rs"])?;
    run_git(origin, &["commit", "-m", "Add Rust target"])
}

fn run_git(cwd: &Path, args: &[&str]) -> io::Result<()> {
    let output = Command::new("git").current_dir(cwd).args(args).output()?;

    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "git {} failed with {:?}",
            args.join(" "),
            output.status.code()
        )))
    }
}

fn expected_check_lines(agent: &str) -> String {
    let judge = if agent == "codex" { "claude" } else { "codex" };
    format!(
        "cyanos: project=demo task=123 check=✅ name=git_installed\n\
cyanos: project=demo task=123 check=✅ name=cargo_installed\n\
cyanos: project=demo task=123 check=✅ name=gh_installed\n\
cyanos: project=demo task=123 check=✅ name=gh_logged_in\n\
cyanos: project=demo task=123 check=✅ name={agent}_installed\n\
cyanos: project=demo task=123 check=✅ name={agent}_logged_in\n\
cyanos: project=demo task=123 check=✅ name={judge}_installed\n\
cyanos: project=demo task=123 check=✅ name={judge}_logged_in\n"
    )
}

#[test]
fn executable_reports_missing_command() -> std::io::Result<()> {
    let output = cyanos_command("cli").output()?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "cyanos: error: missing command\nhint: use 'cyanos init' or 'cyanos run'\n"
    );

    Ok(())
}

#[test]
fn executable_reports_help() -> std::io::Result<()> {
    let output = cyanos_command("cli").arg("--help").output()?;

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    assert!(output.stderr.is_empty());

    Ok(())
}

#[test]
fn executable_accepts_init_command() -> std::io::Result<()> {
    let root = temp_dir("init");
    let home = root.join("home");
    let source = root.join("source");
    fs::create_dir_all(&source)?;
    let output = cyanos_command("cli")
        .env("HOME", &home)
        .current_dir(&source)
        .args(["init", "--project", "owner/repo"])
        .output()?;
    let code = output.status.code();

    assert!(
        output.status.success(),
        "expected successful exit, got {code:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cyanos: initialized project=owner/repo"));
    assert!(stdout.contains("task_readme=false"));
    assert!(output.stderr.is_empty());
    assert!(home.join(".cyanos/projects/owner__repo/origin").is_dir());

    fs::remove_dir_all(root)?;

    Ok(())
}

#[test]
fn executable_prompts_for_missing_init_project_id() -> std::io::Result<()> {
    let root = temp_dir("init-prompt");
    let home = root.join("home");
    let source = root.join("source");
    fs::create_dir_all(&source)?;
    let mut child = cyanos_command("cli")
        .env("HOME", &home)
        .current_dir(&source)
        .args(["init"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| io::Error::other("missing stdin"))?
        .write_all(b"owner/repo\n")?;
    let output = child.wait_with_output()?;

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("project=owner/repo"));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "Project locator:\n"
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_defaults_init_path_to_current_directory() -> std::io::Result<()> {
    let root = temp_dir("init-current-dir");
    let home = root.join("home");
    let source = root.join("source");
    fs::create_dir_all(&source)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .current_dir(&source)
        .args(["init", "--project", "owner/repo"])
        .output()?;

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("project=owner/repo"));
    assert!(home.join(".cyanos/projects/owner__repo/origin").is_dir());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_accepts_run_command() -> std::io::Result<()> {
    let root = fake_dependency_root("run")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let bin = root.join("bin");
    let path = path_with_fake_dependencies(&bin)?;
    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;
    let code = output.status.code();

    assert_eq!(code, Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("task 123 reached terminal failure after 3 run(s)"));
    assert!(stderr.contains("runtime checkpoint tag=task-123-terminal-failure"));
    assert!(stderr.contains("prompt=prompt.2.md"));
    let task_root = home.join(".cyanos/projects/demo/tasks/123");
    assert!(!task_root.join("worktree").exists());
    assert!(task_root.join("runs/1/samples/1/worktree").is_dir());
    assert!(task_root.join("README.md").is_file());
    assert!(task_root.join("program.md").is_file());
    assert!(task_root.join("prompt.md").is_file());
    assert!(task_root.join("prompt.0.md").is_file());
    assert!(task_root.join("prompt.1.md").is_file());
    assert!(task_root.join("prompt.2.md").is_file());
    assert_eq!(
        fs::read_to_string(task_root.join("runs/1/samples/1/summary.md"))?,
        "fake agent output\n"
    );
    assert!(task_root.join("runs/3/samples/1/eval.json").is_file());
    assert!(task_root.join("result.json").is_file());
    let eval_json = fs::read_to_string(task_root.join("runs/3/samples/1/eval.json"))?;
    assert!(eval_json.contains("\"verdict\": \"rejected\""));
    assert!(eval_json.contains("\"structural_evidence\": [\"repository_state: no_changes\"]"));
    assert!(eval_json.contains("No source changes were produced"));
    let result_json = fs::read_to_string(task_root.join("result.json"))?;
    assert!(result_json.contains("\"status\": \"baseline\""));
    assert!(result_json.contains("\"failure_class\": \"verification\""));
    assert!(!home.join(".cyanos/projects/demo/cyanos.lock").exists());
    let runtime_remote = home.join(".cyanos/runtime-remotes/demo.git");
    run_git(
        &runtime_remote,
        &[
            "show-ref",
            "--verify",
            "refs/tags/task-123-terminal-failure",
        ],
    )?;

    fs::remove_dir_all(root)?;

    Ok(())
}

#[test]
fn executable_blocks_incomplete_task_before_agent_execution() -> std::io::Result<()> {
    let root = fake_dependency_root("blocked-intake")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let invalid_body = root.join("invalid-body.md");
    fs::write(&invalid_body, "## Problem\n\nTBD\n")?;
    let gh_log = root.join("gh-comment.log");
    let sentinel = root.join("agent-started");
    let bin = root.join("bin");
    fs::write(
        bin.join("codex"),
        format!(
            "#!/bin/sh\nprompt=$(cat)\nif [ \"$1\" = \"exec\" ]; then\n  touch '{}'\nfi\nexit 0\n",
            sentinel.display()
        ),
    )?;
    #[cfg(unix)]
    set_executable(&bin.join("codex"))?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_ISSUE_BODY_FILE", &invalid_body)
        .env("CYANOS_FAKE_GH_LOG", &gh_log)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "{}{}",
            expected_check_lines("codex"),
            "cyanos: project=demo task=123 state=judge_config worker_agent=codex judge_agent=claude judge_model=best-supported judge_isolation=minimal-read-search-inspect\ncyanos: project=demo task=123 state=loading_task repo=owner/repo\ncyanos: project=demo task=123 state=blocked_intake comment=https://github.com/owner/repo/issues/123#issuecomment-1\ncyanos: project=demo task=123 state=runtime_pushed tag=task-123-blocked-intake\n"
        )
    );
    assert!(output.stderr.is_empty());
    assert!(!sentinel.exists());
    assert!(!home.join(".cyanos/projects/demo/tasks/123").exists());
    let comment = fs::read_to_string(gh_log)?;
    assert!(comment.contains("Section is too vague: Problem"));
    assert!(comment.contains("Missing required section: Expected Behavior"));
    assert!(comment.contains("Missing required section: Verification"));
    let runtime_remote = home.join(".cyanos/runtime-remotes/demo.git");
    run_git(
        &runtime_remote,
        &["show-ref", "--verify", "refs/tags/task-123-blocked-intake"],
    )?;

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_uses_issue_url_when_blocker_comment_has_empty_stdout() -> std::io::Result<()> {
    let root = fake_dependency_root("blocked-intake-empty-comment")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let invalid_body = root.join("invalid-body.md");
    fs::write(&invalid_body, "## Problem\n\nTBD\n")?;
    let bin = root.join("bin");
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_ISSUE_BODY_FILE", &invalid_body)
        .env("CYANOS_FAKE_ISSUE_COMMENT_EMPTY", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("state=blocked_intake comment=https://github.com/owner/repo/issues/123")
    );
    assert!(output.stderr.is_empty());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_reports_github_task_load_failures() -> std::io::Result<()> {
    let root = fake_dependency_root("github-task-failed")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let bin = root.join("bin");
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_ISSUE_VIEW_FAIL", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to load GitHub task"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_rejects_pull_request_as_task_source() -> std::io::Result<()> {
    let root = fake_dependency_root("pull-request-task")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let bin = root.join("bin");
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_TASK_TYPE", "PullRequest")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("resolves to a pull request"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_reports_malformed_github_task_output() -> std::io::Result<()> {
    let root = fake_dependency_root("github-task-malformed")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let bin = root.join("bin");
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_ISSUE_VIEW_MALFORMED", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to load GitHub task"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_blocks_before_agent_when_repository_permission_missing() -> std::io::Result<()> {
    let root = fake_dependency_root("repo-permission")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let sentinel = root.join("agent-started");
    let bin = root.join("bin");
    fs::write(
        bin.join("codex"),
        format!(
            "#!/bin/sh\nprompt=$(cat)\nif [ \"$1\" = \"exec\" ]; then\n  touch '{}'\nfi\nexit 0\n",
            sentinel.display()
        ),
    )?;
    #[cfg(unix)]
    set_executable(&bin.join("codex"))?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_REPO_PERMISSION", "READ")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("current viewerPermission is READ"));
    assert!(stderr.contains("check: ❌ target repository owner/repo write_permission"));
    assert!(stderr.contains("grant write permission"));
    assert!(!sentinel.exists());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_reports_blocker_comment_failures() -> std::io::Result<()> {
    let root = fake_dependency_root("github-comment-failed")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let invalid_body = root.join("invalid-body.md");
    fs::write(&invalid_body, "## Problem\n\nTBD\n")?;
    let bin = root.join("bin");
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_ISSUE_BODY_FILE", &invalid_body)
        .env("CYANOS_FAKE_ISSUE_COMMENT_FAIL", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("failed to post GitHub blocker comment")
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_streams_single_sample_tool_calls_to_stdout() -> std::io::Result<()> {
    let root = fake_dependency_root("single-sample-stream")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let bin = root.join("bin");
    let codex = bin.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\nprompt=$(cat)\nprintf '%s\\n' '{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"cargo check\"}}'\nexit 0\n",
    )?;
    #[cfg(unix)]
    set_executable(&codex)?;
    let path = path_with_fake_dependencies(&bin)?;
    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;
    let code = output.status.code();

    assert_eq!(code, Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[Bash: cargo check]\n[Bash: cargo check]\n[Bash: cargo check]\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("terminal failure"));

    fs::remove_dir_all(root)?;

    Ok(())
}

#[test]
fn executable_records_changed_worktree_as_unpromoted_candidate() -> std::io::Result<()> {
    let root = fake_dependency_root("changed-worktree")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 41)?;
    let bin = root.join("bin");
    let codex = bin.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\nprompt=$(cat)\ncase \"$* $prompt\" in\n  *\"Cyanos code review judge\"*) printf '%s\\n' 'CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote findings=none summary=ok'; exit 0 ;;\nesac\nif [ \"$1\" = \"exec\" ]; then\n  printf 'candidate output\\n'\n  cat > src/lib.rs <<'EOF'\npub fn answer() -> u8 { 42 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn answer_is_42() {\n        assert_eq!(super::answer(), 42);\n    }\n}\nEOF\nfi\nexit 0\n",
    )?;
    #[cfg(unix)]
    set_executable(&codex)?;
    let path = path_with_fake_dependencies(&bin)?;
    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;
    let code = output.status.code();

    assert_eq!(code, Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot open PR for task 123"));
    let task_root = home.join(".cyanos/projects/demo/tasks/123");
    assert_eq!(
        fs::read_to_string(task_root.join("runs/1/samples/1/summary.md"))?,
        "candidate output\n"
    );
    assert!(
        fs::read_to_string(task_root.join("runs/1/samples/1/eval.json"))?
            .contains("\"verdict\": \"accepted\"")
    );
    let eval_json = fs::read_to_string(task_root.join("runs/1/samples/1/eval.json"))?;
    assert!(eval_json.contains("\"structural_evidence\""));
    assert!(eval_json.contains("\"recall_evidence\""));
    assert!(eval_json.contains("cargo_check: passed"));
    assert!(eval_json.contains("cargo_clippy: passed"));
    assert!(eval_json.contains("cargo_test: passed"));
    assert!(eval_json.contains("coverage: 100.00%"));
    assert!(task_root.join("runs/1/samples/1/patch.diff").is_file());
    let result_json = fs::read_to_string(task_root.join("result.json"))?;
    assert!(result_json.contains("\"status\": \"perfect\""));
    assert!(result_json.contains("\"quality_tier\": \"passed\""));
    assert!(result_json.contains("\"failure_class\": \"none\""));
    assert!(task_root.join("ledger.jsonl").is_file());
    let runtime_remote = home.join(".cyanos/runtime-remotes/demo.git");
    run_git(
        &runtime_remote,
        &["show-ref", "--verify", "refs/tags/task-123-best-promoted"],
    )?;

    fs::remove_dir_all(root)?;

    Ok(())
}

#[test]
fn executable_persists_structured_judge_findings_as_recall_evidence() -> std::io::Result<()> {
    let root = fake_dependency_root("structured-judge")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 41)?;
    let bin = root.join("bin");
    write_script(
        &bin.join("codex"),
        r#"#!/bin/sh
	prompt=$(cat)
	if [ "$1" = "exec" ]; then
	  cat > src/lib.rs <<'EOF'
pub fn answer() -> u8 { 42 }

#[cfg(test)]
mod tests {
    #[test]
    fn answer_is_42() {
        assert_eq!(super::answer(), 42);
    }
}
EOF
fi
exit 0
"#,
    )?;
    write_script(
        &bin.join("claude"),
        r#"#!/bin/sh
	prompt=$(cat)
	case "$* $prompt" in
	  *"Cyanos code review judge"*)
    printf '%s\n' 'CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=high next=fix review findings=category=security,severity=major,concern=secret leak,fix=remove secret,evidence=diff,extra=ignored;category=maintainability,severity=minor,concern=messy code,fix=refactor,evidence=diff;category=docs,severity=note,concern=missing docs,fix=add docs,evidence=readme;category=unknown,severity=major,concern=odd behavior,fix=inspect,evidence=judge summary=structured findings'
    exit 0
    ;;
esac
exit 0
"#,
    )?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    let eval_json = fs::read_to_string(
        home.join(".cyanos/projects/demo/tasks/123/runs/1/samples/1/eval.json"),
    )?;
    assert!(
        eval_json.contains(
            "judge: agent=claude model=best-supported isolation=minimal-read-search-inspect verdict=pass rubric=mvp-2026-05-24"
        )
    );
    assert!(eval_json.contains("category=security severity=major"));
    assert!(eval_json.contains("category=maintainability severity=minor"));
    assert!(eval_json.contains("category=docs severity=note"));
    assert!(eval_json.contains("summary=structured findings"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_pushes_and_opens_pr_for_remote_project() -> std::io::Result<()> {
    let root = fake_dependency_root("remote-pr")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 42)?;
    let remote = setup_push_remote(&origin, &root)?;
    let bin = root.join("bin");
    let codex = bin.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\nprompt=$(cat)\ncase \"$* $prompt\" in\n  *\"Cyanos code review judge\"*) printf '%s\\n' 'CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote findings=none summary=ok'; exit 0 ;;\nesac\nif [ \"$1\" = \"exec\" ]; then\n  printf 'candidate output\\n'\n  cat > src/lib.rs <<'EOF'\npub fn answer() -> u8 { 42 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn answer_is_42() {\n        assert_eq!(super::answer(), 42);\n    }\n}\nEOF\nfi\nexit 0\n",
    )?;
    #[cfg(unix)]
    set_executable(&codex)?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;
    let code = output.status.code();

    assert!(
        output.status.success(),
        "expected successful exit, got {code:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("state=pr_opened pr=https://github.com/owner/repo/pull/456"));
    assert!(stdout.contains("state=pr_ready pr=https://github.com/owner/repo/pull/456"));
    assert!(stdout.contains("pr=https://github.com/owner/repo/pull/456"));
    assert!(output.stderr.is_empty());
    run_git(&remote, &["show-ref", "--verify", "refs/heads/cyanos/123"])?;
    run_git(&remote, &["show-ref", "--verify", "refs/heads/feature/123"])?;
    let runtime_remote = home.join(".cyanos/runtime-remotes/demo.git");
    run_git(
        &runtime_remote,
        &["show-ref", "--verify", "refs/tags/task-123-pr-opened"],
    )?;
    run_git(
        &runtime_remote,
        &["show-ref", "--verify", "refs/tags/task-123-pr-ready"],
    )?;

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_adopts_existing_delivery_pr_branch() -> std::io::Result<()> {
    let root = fake_dependency_root("adopt-existing-pr")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 42)?;
    let remote = setup_push_remote(&origin, &root)?;
    let bin = root.join("bin");
    write_script(
        &bin.join("codex"),
        "#!/bin/sh\nprompt=$(cat)\ncase \"$* $prompt\" in\n  *\"Cyanos code review judge\"*) printf '%s\\n' 'CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote findings=none summary=ok'; exit 0 ;;\nesac\nif [ \"$1\" = \"exec\" ]; then\n  cat > src/lib.rs <<'EOF'\npub fn answer() -> u8 { 42 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn answer_is_42() {\n        assert_eq!(super::answer(), 42);\n    }\n}\nEOF\nfi\nexit 0\n",
    )?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_ADOPT_PR", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("state=pr_opened pr=https://github.com/owner/repo/pull/456")
    );
    run_git(
        &remote,
        &[
            "show-ref",
            "--verify",
            "refs/heads/feature/installable-alpha",
        ],
    )?;
    assert!(run_git(&remote, &["show-ref", "--verify", "refs/heads/feature/123"]).is_err());
    let identity =
        fs::read_to_string(home.join(".cyanos/projects/demo/tasks/123/delivery-pr.txt"))?;
    assert!(identity.contains("url=https://github.com/owner/repo/pull/456"));
    assert!(identity.contains("head=feature/installable-alpha"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_resumes_pr_ready_without_restarting_samples() -> std::io::Result<()> {
    let root = fake_dependency_root("resume-pr-ready")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let task_root = home.join(".cyanos/projects/demo/tasks/123");
    fs::create_dir_all(&task_root)?;
    fs::write(
        task_root.join("ledger.jsonl"),
        "{\"task\":\"123\",\"state\":\"pr_ready\",\"summary\":\"ready\"}\n",
    )?;
    fs::write(
        task_root.join("result.json"),
        completed_task_result_json(1, "ready"),
    )?;
    fs::write(
        task_root.join("delivery-pr.txt"),
        "url=https://github.com/owner/repo/pull/456\nhead=feature/installable-alpha\n",
    )?;
    let bin = root.join("bin");
    write_script(
        &bin.join("codex"),
        "#!/bin/sh\ncase \"$1\" in\n  --version) exit 0 ;;\n  login) exit 0 ;;\nesac\nprintf '%s\\n' 'agent should not run during pr_ready resume' >&2\nexit 7\n",
    )?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("state=resuming from=pr_ready last_run=1 next_run=1"));
    assert!(stdout.contains("state=pr_ready pr=https://github.com/owner/repo/pull/456"));
    assert!(output.stderr.is_empty());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_resumes_accepted_sample_without_restarting_samples() -> std::io::Result<()> {
    let root = fake_dependency_root("resume-accepted-sample")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 42)?;
    let remote = setup_push_remote(&origin, &root)?;
    let task_root = home.join(".cyanos/projects/demo/tasks/123");
    let sample_root = task_root.join("runs/1/samples/1");
    fs::create_dir_all(&sample_root)?;
    fs::write(
        task_root.join("ledger.jsonl"),
        "{\"task\":\"123\",\"state\":\"global_best_accepted\",\"summary\":\"accepted\"}\n",
    )?;
    fs::write(
        task_root.join("result.json"),
        completed_task_result_json(1, "promote verified candidate"),
    )?;
    fs::write(
        sample_root.join("eval.json"),
        "{\n  \"verdict\": \"accepted\",\n  \"structural_evidence\": [\"cargo_test: passed\"],\n  \"recall_evidence\": [\"judge: 90%\"],\n  \"findings\": []\n}\n",
    )?;
    fs::write(sample_root.join("summary.md"), "accepted sample")?;
    fs::write(
        sample_root.join("patch.diff"),
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@\n-pub fn answer() -> u8 { 41 }\n+pub fn answer() -> u8 { 42 }\n \n #[cfg(test)]\n mod tests {\n",
    )?;
    let bin = root.join("bin");
    write_script(
        &bin.join("codex"),
        "#!/bin/sh\ncase \"$1\" in\n  --version) exit 0 ;;\n  login) exit 0 ;;\nesac\nprintf '%s\\n' 'agent should not run during accepted sample resume' >&2\nexit 7\n",
    )?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("state=resuming from=promotion last_run=1 next_run=1"));
    assert!(stdout.contains("state=resumed_selected_sample run=1 selected_sample=1"));
    assert!(stdout.contains("state=pr_ready pr=https://github.com/owner/repo/pull/456"));
    assert!(output.stderr.is_empty());
    run_git(&remote, &["show-ref", "--verify", "refs/heads/feature/123"])?;
    let promoted = fs::read_to_string(task_root.join("worktree/src/lib.rs"))?;
    assert!(promoted.contains("pub fn answer() -> u8 { 42 }"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_resumes_evaluated_accepted_sample_before_next_run() -> std::io::Result<()> {
    let root = fake_dependency_root("resume-evaluated-accepted-sample")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 42)?;
    setup_push_remote(&origin, &root)?;
    let task_root = home.join(".cyanos/projects/demo/tasks/123");
    let sample_root = task_root.join("runs/1/samples/1");
    fs::create_dir_all(&sample_root)?;
    fs::write(
        task_root.join("ledger.jsonl"),
        "{\"task\":\"123\",\"state\":\"evaluated_samples\",\"summary\":\"selected run 1 sample 1\"}\n",
    )?;
    fs::write(
        task_root.join("result.json"),
        completed_task_result_json(1, "promote verified candidate"),
    )?;
    fs::write(
        sample_root.join("eval.json"),
        "{\n  \"verdict\": \"accepted\",\n  \"structural_evidence\": [\"cargo_test: passed\"],\n  \"recall_evidence\": [\"judge: 90%\"],\n  \"findings\": []\n}\n",
    )?;
    fs::write(sample_root.join("summary.md"), "accepted sample")?;
    fs::write(
        sample_root.join("patch.diff"),
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@\n-pub fn answer() -> u8 { 41 }\n+pub fn answer() -> u8 { 42 }\n \n #[cfg(test)]\n mod tests {\n",
    )?;
    let path = path_with_fake_dependencies(&root.join("bin"))?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_PR_READINESS_FAIL", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("terminal failure"),
        "stdout:\n{}\nstderr:\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    let ledger = fs::read_to_string(task_root.join("ledger.jsonl"))?;
    assert!(ledger.contains("\"state\":\"global_best_accepted\""));
    assert!(ledger.contains("\"state\":\"pr_feedback\""));
    assert!(ledger.contains("\"state\":\"prompt_evolved\""));
    assert!(task_root.join("prompt.1.md").is_file());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_resumes_pr_feedback_without_restarting_at_run_one() -> std::io::Result<()> {
    let root = fake_dependency_root("resume-pr-feedback")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 41)?;
    setup_push_remote(&origin, &root)?;
    let task_root = home.join(".cyanos/projects/demo/tasks/123");
    fs::create_dir_all(&task_root)?;
    fs::write(
        task_root.join("ledger.jsonl"),
        "{\"task\":\"123\",\"state\":\"pr_feedback\",\"summary\":\"ci failed\"}\n",
    )?;
    fs::write(
        task_root.join("result.json"),
        completed_task_result_json(1, "continue"),
    )?;
    fs::write(
        task_root.join("delivery-pr.txt"),
        "url=https://github.com/owner/repo/pull/456\nhead=feature/123\n",
    )?;
    fs::write(task_root.join("prompt.md"), "resume prompt")?;
    fs::write(task_root.join("program.md"), "outer prompt")?;
    fs::write(task_root.join("prompt.0.md"), "resume prompt")?;
    let bin = root.join("bin");
    write_script(
        &bin.join("codex"),
        "#!/bin/sh\nprompt=$(cat)\ncase \"$* $prompt\" in\n  *\"Cyanos code review judge\"*) printf '%s\\n' 'CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote findings=none summary=ok'; exit 0 ;;\nesac\nif [ \"$1\" = \"exec\" ]; then\n  cat > src/lib.rs <<'EOF'\npub fn answer() -> u8 { 42 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn answer_is_42() {\n        assert_eq!(super::answer(), 42);\n    }\n}\nEOF\nfi\nexit 0\n",
    )?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_PR_READINESS_FAIL", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("terminal failure after 3 run(s)"),
        "stderr:\n{stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout:\n{stdout}");
    assert!(task_root.join("prompt.1.md").is_file());
    assert!(task_root.join("runs/2/samples/1/eval.json").is_file());
    assert!(!task_root.join("runs/1/samples/1/eval.json").exists());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_reports_terminal_failure_when_pr_feedback_resume_is_exhausted() -> std::io::Result<()>
{
    let root = fake_dependency_root("resume-pr-feedback-terminal")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 42)?;
    setup_push_remote(&origin, &root)?;
    let task_root = home.join(".cyanos/projects/demo/tasks/123");
    fs::create_dir_all(&task_root)?;
    fs::write(
        task_root.join("ledger.jsonl"),
        "{\"task\":\"123\",\"state\":\"pr_feedback\",\"summary\":\"ci failed\"}\n",
    )?;
    fs::write(
        task_root.join("result.json"),
        completed_task_result_json(3, "continue"),
    )?;
    fs::write(
        task_root.join("delivery-pr.txt"),
        "url=https://github.com/owner/repo/pull/456\nhead=feature/123\n",
    )?;
    let bin = root.join("bin");
    write_script(
        &bin.join("codex"),
        "#!/bin/sh\ncase \"$1\" in\n  --version) exit 0 ;;\n  login) exit 0 ;;\nesac\nprintf '%s\\n' 'agent should not run during exhausted pr_feedback resume' >&2\nexit 7\n",
    )?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_PR_READINESS_FAIL", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("resumed from pr_feedback but no outer runs remain"),
        "stdout:\n{}\nstderr:\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    let ledger = fs::read_to_string(task_root.join("ledger.jsonl"))?;
    assert!(ledger.contains("\"state\":\"terminal_failure\""));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_reports_terminal_failure_after_pr_feedback_retries() -> std::io::Result<()> {
    let root = fake_dependency_root("pr-feedback-terminal")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 42)?;
    let remote = setup_push_remote(&origin, &root)?;
    let bin = root.join("bin");
    let codex = bin.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\nprompt=$(cat)\ncase \"$* $prompt\" in\n  *\"Cyanos code review judge\"*) printf '%s\\n' 'CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote findings=none summary=ok'; exit 0 ;;\nesac\nif [ \"$1\" = \"exec\" ]; then\n  printf 'candidate output\\n'\n  i=1\n  while [ -e \"evidence-$i.txt\" ]; do i=$((i + 1)); done\n  printf 'evidence\\n' > \"evidence-$i.txt\"\nfi\nexit 0\n",
    )?;
    #[cfg(unix)]
    set_executable(&codex)?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_PR_READINESS_FAIL", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("PR readiness still has unresolved feedback"));
    assert!(stderr.contains("runtime checkpoint tag=task-123-terminal-failure"));
    run_git(&remote, &["show-ref", "--verify", "refs/heads/cyanos/123"])?;
    let runtime_remote = home.join(".cyanos/runtime-remotes/demo.git");
    run_git(
        &runtime_remote,
        &[
            "show-ref",
            "--verify",
            "refs/tags/task-123-terminal-failure",
        ],
    )?;

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_rejects_candidate_when_coverage_cannot_be_measured() -> std::io::Result<()> {
    let root = fake_dependency_root("coverage-zero")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let bin = root.join("bin");
    fs::write(
        bin.join("cargo"),
        "#!/bin/sh\ncase \"$1\" in\n  check|clippy|test) exit 0 ;;\n  llvm-cov) exit 6 ;;\nesac\nexit 0\n",
    )?;
    #[cfg(unix)]
    set_executable(&bin.join("cargo"))?;
    let codex = bin.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\nprompt=$(cat)\nif [ \"$1\" = \"exec\" ]; then\n  printf 'candidate output\\n'\n  printf 'changed\\n' > IMPLEMENTED.txt\nfi\nexit 0\n",
    )?;
    #[cfg(unix)]
    set_executable(&codex)?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FORCE_COVERAGE_VERIFIER", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;
    let code = output.status.code();

    assert_eq!(code, Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("terminal failure"));
    assert!(stderr.contains("prompt=prompt.2.md"));
    let eval_json = fs::read_to_string(
        home.join(".cyanos/projects/demo/tasks/123/runs/3/samples/1/eval.json"),
    )?;
    assert!(eval_json.contains("coverage recall is zero"));
    assert!(eval_json.contains("coverage: 0.00%"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_reports_judge_command_failures_after_verified_candidate() -> std::io::Result<()> {
    let root = fake_dependency_root("judge-failure")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 42)?;
    let bin = root.join("bin");
    write_script(
        &bin.join("codex"),
        r#"#!/bin/sh
	prompt=$(cat)
	if [ "$1" = "exec" ]; then
  cat > src/lib.rs <<'EOF'
pub fn answer() -> u8 { 42 }

#[cfg(test)]
mod tests {
    #[test]
    fn answer_is_42() {
        assert_eq!(super::answer(), 42);
    }
}
EOF
fi
exit 0
"#,
    )?;
    write_script(
        &bin.join("claude"),
        r#"#!/bin/sh
	prompt=$(cat)
	case "$* $prompt" in
  *"Cyanos code review judge"*) printf '%s\n' 'judge unavailable' >&2; exit 7 ;;
esac
exit 0
"#,
    )?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("agent command"), "stderr:\n{stderr}");
    assert!(stderr.contains("judge unavailable"), "stderr:\n{stderr}");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_retries_and_reports_agent_failures() -> std::io::Result<()> {
    let root = fake_dependency_root("agent-failure")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let bin = root.join("bin");
    fs::write(
        bin.join("codex"),
        "#!/bin/sh\nprompt=$(cat)\nif [ \"$1\" != \"exec\" ]; then exit 0; fi\nprintf 'agent denied\\n' >&2\nexit 7\n",
    )?;
    #[cfg(unix)]
    set_executable(&bin.join("codex"))?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("agent denied"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_reports_pr_create_failures_after_verified_candidate() -> std::io::Result<()> {
    let root = fake_dependency_root("pr-create-failed")?;
    let home = root.join("home");
    let origin = setup_managed_origin(&home, "demo")?;
    write_smoke_rust_target(&origin, 42)?;
    let _remote = setup_push_remote(&origin, &root)?;
    let bin = root.join("bin");
    let codex = bin.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\nprompt=$(cat)\ncase \"$* $prompt\" in\n  *\"Cyanos code review judge\"*) printf '%s\\n' 'CYANOS_JUDGE verdict=pass rubric=mvp-2026-05-24 risk=low next=promote findings=none summary=ok'; exit 0 ;;\nesac\nif [ \"$1\" = \"exec\" ]; then\n  cat > src/lib.rs <<'EOF'\npub fn answer() -> u8 { 42 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn answer_is_42() {\n        assert_eq!(super::answer(), 42);\n    }\n}\nEOF\nfi\nexit 0\n",
    )?;
    #[cfg(unix)]
    set_executable(&codex)?;
    let path = path_with_fake_dependencies(&bin)?;

    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .env("CYANOS_FAKE_PR_CREATE_FAIL", "1")
        .args(["run", "--project", "demo", "--task", "123"])
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("gh pr create failed"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn executable_accepts_sample_count() -> std::io::Result<()> {
    let root = fake_dependency_root("samples")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let bin = root.join("bin");
    fs::write(
        bin.join("codex"),
        "#!/bin/sh\nprompt=$(cat)\nprintf '%s\\n' '{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"cargo check\"}}'\nexit 0\n",
    )?;
    #[cfg(unix)]
    set_executable(&bin.join("codex"))?;
    let path = path_with_fake_dependencies(&bin)?;
    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args([
            "run",
            "--project",
            "demo",
            "--task",
            "123",
            "--samples",
            "2",
        ])
        .output()?;
    let code = output.status.code();

    assert_eq!(code, Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sample 1 [Bash: cargo check]\n"));
    assert!(stdout.contains("sample 2 [Bash: cargo check]\n"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("terminal failure"));
    let task_root = home.join(".cyanos/projects/demo/tasks/123");
    assert!(task_root.join("runs/1/samples/1/worktree").is_dir());
    assert!(task_root.join("runs/1/samples/2/worktree").is_dir());
    assert!(task_root.join("runs/3/samples/1/eval.json").is_file());
    assert!(task_root.join("runs/3/samples/2/eval.json").is_file());

    fs::remove_dir_all(root)?;

    Ok(())
}

#[test]
fn executable_accepts_agent_and_model_options() -> std::io::Result<()> {
    let root = fake_dependency_root("agent-model")?;
    let home = root.join("home");
    setup_managed_origin(&home, "demo")?;
    let bin = root.join("bin");
    let path = path_with_fake_dependencies(&bin)?;
    let output = cyanos_command("cli")
        .env("HOME", &home)
        .env("PATH", path)
        .args([
            "run",
            "--project",
            "demo",
            "--task",
            "123",
            "--agent",
            "claude",
            "--model",
            "sonnet",
        ])
        .output()?;
    let code = output.status.code();

    assert_eq!(code, Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("terminal failure"));
    assert!(stderr.contains("agent=claude model=sonnet"));

    fs::remove_dir_all(root)?;

    Ok(())
}

#[test]
fn executable_reports_usage_errors_on_stderr() -> std::io::Result<()> {
    let output = cyanos_command("cli")
        .args(["run", "--project", "demo", "--agent", "unknown"])
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "cyanos: error: unknown agent: unknown\nhint: supported agents: claude, codex\n"
    );

    Ok(())
}
