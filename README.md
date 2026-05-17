# Cyanos

Cyanos is an autonomous, model-agnostic coding agent CLI that turns resolved written requirements and GitHub issues into user-mergeable product changes.

The project wraps existing coding agent CLIs behind an adapter layer. Use `--agent <agent>` to choose Claude or Codex, and `--model <model>` to override the selected adapter's default best-supported model.

The architecture uses a self-evolving two-layer loop. The inner loop is Sample-Select-Distill: run N isolated samples, score them, and promote the best result. The outer loop evaluates the global best, then updates `prompt.md` once per run from a concrete hypothesis about the best sample's defect.

The MVP run path works on one selected task id. If the fetched issue is unclear, Cyanos posts asynchronous blocker comments on the issue, keeps that task blocked, and starts the evolution loop only after the requirement is executable. Users do not steer the agent through an interactive CLI session.

This installable alpha establishes the CLI, repository policy, runtime layout, agent adapter boundary, and first single-task orchestrator path. Multi-issue daemon scheduling is outside the MVP scope.

After a task enters the evolution loop, opening a PR is not completion. Cyanos keeps working until the PR fully implements the user's intent and has no unresolved comments or concerns. Cyanos must not merge the PR; the task ends only after the user merges it.

For GitHub-based projects, the task id is the GitHub issue id. Issue `123` uses `tasks/123/`, `cyanos/123`, and `feature/123`.

## Commands

- `cyanos init --project <repo-locator>` initializes a managed project for the current GitHub-backed repository. Use `owner/repo` for github.com or a full repository URL for GitHub Enterprise.
- `cyanos run --project <repo-locator> --task <task-id> [--agent <agent>] [--model <model>] [--samples <n>]` runs one selected task.

The supported installation path is Homebrew: `brew install johnsonlee/tap/cyanos`.
