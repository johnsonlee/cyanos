# Coding Guidelines

Cyanos is a coding agent CLI that accepts command-line arguments. All pull requests must aim for Rust community best practice, not merely passing code. A PR is acceptable only when the implementation is idiomatic, maintainable, tested, verified with the relevant commands, and ready for the user to merge without comments or concerns.

## Required PR Standard

Every PR must:

- Be opened as a pull request; direct feature commits to `main` are not allowed.
- Contain exactly one feature or coherent behavior change.
- Contain exactly one commit.
- Use a title that describes the actual change.
- Write all documentation, PR titles, and PR bodies in English. Emoji are allowed when useful, but non-English prose and section headings are not allowed.
- Keep the change scoped to one coherent behavior or refactor.
- Prefer simple, explicit Rust over clever abstractions.
- Compile cleanly with `cargo check`.
- Pass `cargo fmt --check`.
- Pass `cargo clippy --all-targets --all-features -- -D warnings`.
- Pass `cargo llvm-cov --all-targets --all-features --fail-under-lines 95`.
- Pass `cargo deny check`.
- Pass `cargo machete`.
- Include meaningful tests for new behavior and regressions.
- Include benchmark results in every PR.
- Avoid introducing hidden global state, silent fallbacks, or unverified assumptions.
- Explain verification in the PR description.
- Attach workflow test and coverage results to the PR as a comment after CI runs.
- Resolve every review comment and concern before the task can be considered complete.

If a command cannot be run, the PR must say why and describe the remaining risk.

The PR body must include: concrete requirement, implementation method, architecture and functional impact, test and verification method, and test results. Test results must include functional tests, benchmark tests, and end-to-end tests. The CI metadata check enforces required sections, placeholder replacement, and the single-commit rule.

Opening a PR is not task completion. For Cyanos-managed tasks, the task ends only after the user merges the PR. Cyanos must never merge the PR itself. Once a task enters the evolution loop, the only acceptable goal is a PR that fully implements the user's intent and can be merged directly by the user.

## Automated Quality Gate

The repository enforces Rust bad-practice checks through checked-in tool configuration:

- `Cargo.toml` defines rustc and Clippy lints based on Microsoft Pragmatic Rust Guidelines and local CLI rules, with strict rules for unsafe code, unwraps, panics, debug macros, wildcard imports, unchecked indexing, ignored errors, and stdout/stderr printing.
- `clippy.toml` tightens test behavior and complexity thresholds.
- `tools/source-lint` is the Rust policy verifier for repo-specific guideline gaps that Clippy does not cover directly. It rejects inline semantic strings, magic numbers, `#[allow(...)]`, `#[expect(...)]` without `reason`, public glob re-exports, `static` state in production `src`, architecture boundary violations, and documentation with non-English letters.
- `deny.toml` checks dependency advisories, licenses, duplicate versions, wildcard dependencies, and unknown sources.
- `.github/workflows/ci.yml` runs build, lint, test, coverage, dependency, and PR metadata gates for every PR and push to `main`.
- `.github/scripts/validate-pr-metadata.sh` rejects PRs with multiple commits, generic titles, non-English titles or bodies, missing required body sections, or unfilled template placeholders.
- `.github/scripts/comment-pr-report.sh` posts or updates the PR test and coverage report comment after workflow execution.
- `Makefile` provides `make verify` for local pre-PR validation.

Automated linting catches static bad practices; architecture fit, user-intent alignment, and prompt-loop quality still require human or agent review.

## Release Workflow

Releases are created only by pushing `v*` tags that match `Cargo.toml` version, for example `v0.1.0`. The release workflow must pass build, lint, test, coverage, dependency audit, unused dependency checks, artifact packaging, GitHub Release publication, and Homebrew tap update. The user installation path is Homebrew only and must install prebuilt executable artifacts, not build from source. Do not create manual release artifacts outside GitHub Actions.

## Rust Style

Use `rustfmt` defaults and follow Microsoft Pragmatic Rust Guidelines unless a repo convention is stricter. Do not hand-format around formatter output. Follow standard naming:

- Modules, functions, variables, and tests: `snake_case`.
- Types, traits, and enum variants: `PascalCase`.
- Constants and statics: `SCREAMING_SNAKE_CASE`.

Keep modules cohesive. A module should own a clear concept or workflow, not become a dumping ground for helpers. Prefer private functions until an API is needed by another module. Public APIs must be intentional and documented when behavior, invariants, or failure modes are not obvious.

Each source file must have exactly one subject. The subject is the primary concept named by the file path, such as `agent/registry.rs`, `agent/adapters/claude.rs`, or `workspace.rs`. Do not mix multiple concrete adapters, unrelated data models, orchestration workflows, and utility clusters in one file. If a file needs several peer concepts to be understandable, split it into a directory module and one file per subject.

Lint overrides must use `#[expect(lint_name, reason = "...")]`; do not use `#[allow(...)]` in handwritten source. Magic values must be named constants with clear names and comments when the reason is non-obvious.

Good:

```rust
impl PromptStore {
    pub fn inner_prompt_path(&self) -> Result<PathBuf, PromptError> {
        self.prompt_dir.join("inner.md").canonicalize().map_err(PromptError::from)
    }
}
```

Bad:

```rust
pub fn do_stuff(path: String) -> String {
    std::fs::read_to_string(path).unwrap()
}
```

## Error Handling

Use `Result<T, E>` for recoverable failures. Use precise error types for library-like boundaries and contextual errors for application boundaries. Prefer `thiserror` for domain errors and `anyhow` only at top-level orchestration layers where rich context matters more than matching on variants.

Do not use `unwrap()`, `expect()`, or `panic!()` in production paths. Tests should return `Result` or use explicit assertions instead of bypassing lint rules.

All fallible operations must propagate or handle errors explicitly. Do not discard errors with `ok()`, empty `match` arms, ignored `Result`s, broad defaults, or best-effort fallbacks that hide failure. If the program cannot complete the requested operation, return an error with context.

CLI-facing errors must flow through the unified error handling module in `src/error.rs`. Domain modules return typed errors; the CLI boundary maps them into user-friendly messages, hints, and exit codes. Do not format ad hoc user-facing errors in feature modules.

Always preserve context:

```rust
let config = load_config(path)
    .with_context(|| format!("failed to load config from {}", path.display()))?;
```

Avoid:

- Swallowing errors with `ok()` or `unwrap_or_default()`.
- Returning `String` as an error type.
- Logging an error and then continuing with invalid state.

## CLI Behavior

Cyanos is a CLI program and must follow CLI best practice:

- Expose only the stable top-level commands `init` and `run` until a new command has a documented lifecycle.
- `init` owns project setup under `~/.cyanos/projects/<project-id>/origin`; `run` starts one long-lived daemon watcher that claims tasks from the configured source and executes them under `~/.cyanos/projects/<project-id>/tasks/<task-id>/`.
- Gate every fetched issue through asynchronous requirement intake before evolution. If the requirement is unclear, post blocker comments on the issue, keep that task blocked, and continue daemon watching; do not ask for answers in a CLI prompt or inside an agent loop.
- Once a task enters the evolution loop, optimize only for user-mergeable completion. Do not treat PR creation, partial passing checks, or unresolved review feedback as success.
- For GitHub-based projects, the task id is the GitHub issue id. Issue `123` must use `tasks/123/`, `cyanos/123`, and `feature/123`. Cyanos must not generate task ids; non-GitHub task sources must provide their own durable id.
- Check dependencies before orchestration: `git` installed, `gh` installed and logged in, selected agent CLI installed and logged in.
- Hold a project lock before mutating `~/.cyanos/projects/<project-id>/`; one project may have only one active Cyanos agent.
- Use stable runtime paths with unpadded numeric run and sample ids: `runs/1/` and `runs/1/samples/1/`.
- Treat runtime `worktree/` directories as real Git worktrees. Do not ignore or clean them as disposable artifacts.
- Keep Cyanos runtime artifacts outside the user repository. The `cyanos/<task-id>` branch stores promoted source-code evolution history, and `feature/<task-id>` stores PR-ready source code. Neither branch stores ledgers, prompts, eval results, or sample transcripts.
- Keep `stdout` for successful command output and machine-readable data; write diagnostics, warnings, and errors to `stderr`.
- Use stable, documented exit codes: `0` for success, `2` for usage errors, and `1` for runtime failures.
- Provide user-friendly error messages with one clear cause and, when useful, one actionable hint.
- Preserve detailed error sources internally even when the displayed message is concise.
- Support predictable non-interactive execution by default; prompts must be explicit opt-in.
- Keep argument semantics stable. Additive flags are preferred; breaking flag behavior requires a migration note.
- Provide `--help` and `--version` before release, and keep examples accurate.
- Prefer explicit output modes such as `--json` for automation instead of parsing human text.
- Make verbosity intentional through flags such as `--verbose` and `--quiet`; do not print noisy logs by default.
- Respect terminal conventions such as `NO_COLOR`, color only on TTY, and no control characters in non-TTY output.
- Treat filesystem writes as transactional when possible: write to a temporary path, verify, then promote.
- Handle cancellation and timeouts cleanly. Child agent processes must not be orphaned.

## Ownership, Borrowing, and Data Modeling

Borrow by default. Take `&str`, `&Path`, or slices when ownership is not required. Take owned values only when storing them, moving them across tasks, or transforming them into owned state.

Prefer domain types over loosely typed strings and maps. If a value has meaning, model it:

```rust
pub struct AgentId(String);
pub struct EvaluationScore(f32);
```

Avoid stringly typed APIs such as `HashMap<String, String>` when keys and values have stable meaning. Avoid unnecessary `clone()` calls; when cloning is needed, make it visible and cheap or explain why ownership must split.

## API Design

Design APIs around behavior and invariants, not internal data layout. Constructors should validate inputs when invalid states are possible. Prefer small structs with explicit fields over tuples once fields need names. Prefer enums for closed state machines.

Object-oriented design is mandatory. In Rust, that means structs own state, traits define behavior, and collaborators communicate through typed methods. Modules must not become bags of free functions. Procedural workflows that pass loose data through step functions are prohibited. The model-agnostic agent layer must define shared request and command objects, while Claude, Codex, Gemini, and future CLIs live behind independent adapters.

Layers above `agent` must not import concrete adapters or hard-code agent-specific model behavior. The two-layer loop and eval code may depend on `AgentRuntime`, `AgentTurn`, and `AgentOutput`; they must not call a specific agent CLI or select a specific model directly.

Prompt ownership follows the same boundary. `program.md` is the prompt-revision task prompt, `prompt.md` is the coding task prompt embedded at build time, and runtime prompts must be written as immutable snapshots named `prompt.<N>.md` under the task directory. CLI-started agents must be role-blind: prompts must not explain Cyanos topology, orchestration layers, sample identity, scoring, or PR readiness decisions. Code must load prompts through typed prompt objects instead of ad hoc file reads.

For the nested loop, represent state transitions explicitly. Inner SSD runs must model sample identity, sample origin, score, selection, and global best as typed objects. The outer loop may write one new prompt snapshot per SSD run, based only on the global best evaluation. Avoid boolean parameter APIs such as `run_agent(true, false)`. Use typed options or enums instead.

Good:

```rust
enum VerificationMode {
    Fast,
    Full,
}
```

Bad:

```rust
run_verification(true, false);
```

## Async and Concurrency

Use async only when the operation is naturally asynchronous. Do not add async to pure CPU or local state logic. Never block an async executor with long synchronous work; use appropriate blocking task APIs when needed.

Shared mutable state must be explicit and justified. Prefer message passing, immutable snapshots, or scoped ownership over `Arc<Mutex<_>>`. When locks are necessary, keep lock scope small and never hold a lock across `.await`.

## Testing

Tests must assert behavior, not implementation details. Unit tests belong near the code under `#[cfg(test)]`; integration tests belong in `tests/*.rs`. Name tests by behavior, for example `rejects_missing_prompt` or `updates_inner_prompt_after_failed_eval`.

Every bug fix should add a regression test. For agent behavior, test the state transition, prompt update decision, and verification result separately when possible. Use fixtures only when they clarify intent; keep them small and readable.

Coverage is a merge gate. Keep line coverage at or above 95% with `cargo llvm-cov --all-targets --all-features --fail-under-lines 95`. This is the bottom line for every PR.

Avoid:

- Snapshot tests for unstable or incidental formatting.
- Tests that only check that a function returns `Ok(())`.
- Large fixtures that hide the behavior under test.

## Logging and Observability

Use structured logging with `tracing` when runtime visibility is needed. Logs should explain what happened and include stable identifiers such as agent id, task id, prompt version, or evaluation id. Do not log secrets, raw credentials, or full user prompts unless explicitly safe and necessary.

Avoid `println!` in production code except for intentional CLI output.

## Dependencies

Add dependencies conservatively. A dependency must pay for its compile time, maintenance surface, and security risk. Prefer well-maintained crates with active releases, clear licenses, and strong community adoption.

Before adding a crate, check whether the standard library or an existing dependency already solves the problem. Do not add dependencies for trivial helpers.

## Unsafe Code

`unsafe` is prohibited unless there is a clear, measured need that cannot be met safely. Any `unsafe` block must include a `SAFETY:` comment explaining the invariants that make it sound, and tests must cover the safe wrapper around it.

## Performance

Start with clear code, then optimize from evidence. Every PR must include benchmark results in the PR body: command, environment, baseline result, new result, and interpretation. Use `cargo bench` for benchmark harnesses and `hyperfine` for CLI behavior when appropriate.

No benchmark result means no merge. Treat changes that affect execution loops, prompt evaluation, file traversal, concurrency, network calls, dependency behavior, or user-visible latency as performance-sensitive. Prefer streaming or incremental processing for large inputs.

## Configuration and Secrets

Configuration must be explicit and discoverable. Prefer typed config structs over ad hoc environment reads scattered through the code. Read environment variables at the boundary, validate them once, and pass typed values inward.

Never commit secrets, tokens, local credentials, or generated binaries. Example config files are allowed when values are clearly fake.

## Common Rust Bad Practices to Avoid

Avoid these patterns in PRs:

- `unwrap()` or `expect()` in production control flow.
- Catch-all `Box<dyn Error>` in domain layers.
- Public fields that allow invalid state.
- Files that contain multiple subjects or multiple concrete implementations that should evolve independently.
- Large functions that mix parsing, execution, logging, and output formatting.
- Procedure-oriented modules that expose workflow functions instead of objects.
- Boolean flags that encode modes or state transitions.
- Unbounded retries without backoff, limits, or observability.
- Silent defaults that hide configuration mistakes.
- Cloning to satisfy the borrow checker without understanding ownership.
- Holding locks across `.await`.
- Logging sensitive user data.
- Adding abstractions before there are at least two real use cases.
- Mixing unrelated features in one PR.
- Opening a feature PR with multiple commits.
- Submitting performance-sensitive changes without benchmark results.

## Review Checklist

Before opening a PR, verify:

- The implementation matches the stated user intent.
- Cyanos owns orchestration, verification, evaluation, scoring, and PR readiness; CLI-started agents receive only the task input they need.
- Errors are typed or contextual and are not silently discarded.
- Tests cover success, failure, and important edge cases.
- `make verify` or the equivalent individual commands have been run.
- The PR contains one feature, one commit, and a title that matches the change.
- The PR body includes requirement, implementation, impact, verification method, and functional, benchmark, and E2E results.
- The PR description states what changed, why, and how it was verified.
