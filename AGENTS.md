# Repository Guidelines

## Project Structure & Module Organization

Cyanos is an autonomous coding agent CLI with `init` and `run` commands. Keep the layout conventional: crate source in `src/`, integration tests in `tests/`, examples in `examples/`, and benchmarks in `benches/`. Runtime state belongs under `~/.cyanos/projects/<project-id>/`, never in `target/` or ad hoc temp paths.

## Architecture Overview

Cyanos is autonomous and model-agnostic: users define requirements through GitHub issues or written task briefs, and one long-lived Cyanos daemon owns the loop that delivers user-mergeable PRs. The watcher must clarify unclear issues through issue comments and wait until those clarifications are answered or resolved before evolution starts. Once a task enters the evolution loop, the only goal is a PR that fully implements the user's intent and can be merged by the user without comments or concerns. Cyanos must not merge the PR itself. For GitHub issue tasks, the task id is the issue id. It wraps coding agent CLIs through an agent abstraction layer. Each CLI must have an independent adapter. Select the adapter with `cyanos run --project <project-id> --agent <agent>` and optionally set `--model <model>`; by default, each adapter uses its best supported model. The inner loop is SSD: run N isolated samples, score them with tests, benchmarks, and LLM judge output, then promote the best by worktree swap.

## Distribution & Release

The only supported user installation path is Homebrew. Releases are triggered by version tags such as `v0.1.0`, must pass the release workflow, publish prebuilt executables, and update [`johnsonlee/homebrew-tap`](https://github.com/johnsonlee/homebrew-tap).

## Build, Test, and Development Commands

Use the checked-in Rust toolchain:

- `cargo check` for type validation.
- `cargo build` to compile.
- `cargo test` for unit and integration tests.
- `make verify` for formatting, Clippy, tests, coverage, dependency audit, and unused dependency checks.

Do not report a command as passing unless it ran successfully.

## Coding Style & Naming Conventions

Use `rustfmt` defaults and Rust OOP: structs own state, traits define behavior, and adapters encapsulate agent-specific logic. Avoid process-only modules. Follow standard Rust naming: `snake_case` for functions and variables, `PascalCase` for types and traits, and `SCREAMING_SNAKE_CASE` for constants.

## Testing Guidelines

Place unit tests next to covered code under `#[cfg(test)]`, and public API tests in `tests/*.rs`. Name tests for observable behavior, for example `parses_empty_input` or `rejects_invalid_config`. Line coverage must stay at or above 95%.

## Commit & Pull Request Guidelines

Use one feature per pull request and one commit per PR. Titles must match the actual change, for example `Add prompt evaluation loop`. PR bodies must include requirement, implementation, architecture and functional impact, verification method, and test results covering functional, benchmark, and end-to-end tests.

## Security & Configuration

Do not commit secrets, local credentials, generated binaries, or `target/` contents. Prefer example config files over real local config, and document required environment variables near the code that reads them.
