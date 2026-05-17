# Architecture

Cyanos is an autonomous, model-agnostic coding agent CLI. Its job is to turn a written requirement into a PR that fully satisfies the user's intent and can be merged by the user without reviewer comments or concerns.

## Problem

Existing coding agent CLIs are powerful interactive tools, but they still depend on a human operator to steer the conversation, notice drift, rerun verification, compare alternatives, ask follow-up instructions, and decide when the output is PR-ready.

That makes the human the orchestration loop. Cyanos moves that loop into the system. The user defines the requirement and project context; Cyanos runs agent samples, verifies output, evolves prompts, preserves history, delivers a pull request with evidence, and keeps working until the PR is ready for the user to merge.

## Goal

The product goal is autonomous requirement-to-user-mergeable-PR delivery without interactive agent babysitting. The MVP workflow starts by initializing Cyanos for a GitHub-backed local repository and running one selected task id: `cyanos run --project <repo-locator> --task <task-id>`. For GitHub-backed projects that task id is the issue id. Cyanos turns ambiguity into issue-native blocker comments, runs the evolution loop only after the requirement is executable, and stops at a PR that is ready for the user to merge.

The architecture goal is to make autonomous delivery reliable across underlying coding agents. The central design choice is a two-layer evolution loop over an agent abstraction layer. Cyanos owns orchestration, isolation, verification, scoring, prompt evolution, branch promotion, PR readiness, and review-comment resolution. Concrete tools such as Claude and Codex remain replaceable execution backends behind adapters.

## Non-Goals

- Cyanos does not remove the user's authority to review and merge; it treats every review comment or concern as a failed readiness signal that must be resolved before `pr_ready`.
- Cyanos does not make one coding agent mandatory; concrete agent CLIs are interchangeable adapters.
- Cyanos does not store user repository runtime artifacts in the repository history; runtime evidence stays in Cyanos-managed storage.

## Design Principles

- Intent over command execution: the system optimizes for a user-mergeable outcome that satisfies the user's intent, not a single successful subprocess call.
- Autonomy over chat steering: after the requirement is defined, Cyanos owns the iteration loop until it can present a verified PR candidate.
- User-controlled merge: opening a PR is an intermediate artifact, and Cyanos must not merge it; the MVP run lifecycle ends at `pr_ready`.
- Model agnostic by construction: loop and eval code depend on `AgentRuntime`, `AgentTurn`, and `AgentOutput`, never on a concrete agent CLI or model.
- Isolation before comparison: every sample runs in its own worktree so selection is based on evidence, not shared side effects.
- Verification as a gate: structural checks, at least one recall category, and independent code-review findings form the composite score. Benchmark and E2E signals are optional recall categories when the task config defines them.
- Evolution is inspectable: promoted source states are committed, while runtime artifacts stay in Cyanos-managed storage.

## Key Flow

1. `cyanos init --project <repo-locator>` creates managed runtime state for the target repository locator and stores typed project config inferred from the local GitHub-backed repository.
2. `cyanos run --project <repo-locator> --task <task-id>` acquires the project lock and loads that selected task from GitHub.
3. Cyanos validates the task brief sections: `Problem`, `Expected Behavior`, `Scope`, `Acceptance Criteria`, and `Verification`.
4. Cyanos also enforces readability: any task section with more than 5 bullet items must group those bullets under subheadings, otherwise intake blocks before samples start.
5. If the requirement is unclear or unreadable, Cyanos posts asynchronous blocker comments on the issue, records `blocked_intake`, and does not start any coding-agent sample.
6. Once the requirement is clear, Cyanos freezes the resolved task brief into the task runtime directory.
7. The orchestrator runs the two-layer evolution loop with the sole goal of producing a PR that fully implements the user's intent and can be merged by the user without comments or concerns.
8. Cyanos commits promoted source evolution to `cyanos/<task-id>`.
9. Cyanos prepares final PR code on `feature/<task-id>`, opens or updates a pull request, waits for workflow and review readiness signals, resolves all comments and concerns, records `pr_ready`, pushes the runtime checkpoint, and stops.

## System Context

```text
                             +-----------------------+
                             | Person                |
                             | Developer             |
                             | Files issues, reviews |
                             +-----------+-----------+
                                         |
                                         | runs one selected task
                                         v
+-----------------------+    +-----------------------+    +-----------------------+
| External System       |    | System                |    | External System       |
| GitHub                |<-->| Cyanos CLI           |--->| Coding Agent CLI      |
| Issues, PRs, CI       |    | Autonomous runner     |    | Claude/Codex          |
+-----------------------+    +-----------+-----------+    +-----------------------+
                                         |
                                         | creates worktrees and task branches
                                         v
                             +-----------------------+
                             | External System       |
                             | Target Repository     |
                             | Git worktrees/refs    |
                             +-----------------------+
                                         ^
                                         |
                                         | separate from pushed runtime evidence
                                         |
                             +-----------+-----------+
                             | External System       |
                             | Runtime Repository    |
                             | commits/tags          |
                             +-----------------------+

Relationships:

GitHub <-> Cyanos CLI: watches issues, posts blocker comments, opens PRs, waits for checks.
Cyanos CLI -> Coding Agent CLI: runs isolated samples through the adapter boundary.
Cyanos CLI -> Target Repository: creates worktrees and commits `cyanos/*` and `feature/*`.
Cyanos CLI -> Runtime Repository: pushes versioned runtime checkpoints and tags, never target source refs.
```

## Container View

```text
Software System: Cyanos

+-----------------------------------------------------------------------+
| Cyanos                                                                |
|                                                                       |
|  +-------------------------------------+     +---------------------+  |
|  | Container                           |     | Container           |  |
|  | CLI / Daemon                        |<--->| Runtime Store       |  |
|  | Rust executable                     |     | ~/.cyanos/projects  |  |
|  | init, run, evolve, verify PR        |     | local runtime data  |  |
|  +-------------------------------------+     +---------------------+  |
|                                                                       |
+-----------------------------------------------------------------------+

External collaborators:

+--------------------+   +--------------------+   +--------------------+
| Person             |   | External System    |   | External System    |
| Developer          |   | GitHub             |   | Coding Agent CLI   |
| Runs task          |   | Issues, PRs, CI    |   | Claude/Codex       |
+--------------------+   +--------------------+   +--------------------+

+--------------------+
| External System    |
| Target Git Repo    |
| refs/worktrees     |
+--------------------+

+--------------------+
| External System    |
| Runtime Git Repo   |
| checkpoint refs    |
+--------------------+
```

Cyanos has two containers in this view: the Rust CLI / Daemon application and the local Runtime Store under `~/.cyanos/projects`. The Runtime Store is the local working copy for runtime evidence; durable runtime evidence is committed, tagged, and pushed to the resolved Runtime Git Repo. `Task Loader`, `Requirement Intake`, `Orchestrator`, `Agent Adapter Layer`, `Verifier / Scorer`, and `Sample Event Renderer` are components inside the CLI / Daemon container, so they belong in the Component View, not in this Container View.

Relationships:

1. Developer starts the CLI / Daemon for a configured project and selected task id.
2. CLI / Daemon loads the selected GitHub issue, posts blocker comments, opens PRs, waits for CI and review readiness, and stops at `pr_ready`.
3. CLI / Daemon invokes Coding Agent CLI processes through the adapter boundary.
4. CLI / Daemon reads and writes Runtime Store files: task briefs, prompt snapshots, ledgers, sample metadata, eval results, patches, and PR readiness evidence.
5. CLI / Daemon fetches, creates worktrees, commits `cyanos/*`, and prepares `feature/*` in the Target Git Repo.
6. CLI / Daemon commits runtime state, creates durable checkpoint tags, and pushes them only to the Runtime Git Repo.

## Component View

```text
Component View: CLI / Daemon

+----------------+  +----------------+  +----------------+
| CLI Boundary   |->| Task Loader    |->| Intake Gate    |
+----------------+  +----------------+  +-------+--------+
                                             |
                                             v
+----------------+  +----------------+  +----------------+  +----------------+
| Eval Builder   |->| Prompt Evolver |->| Orchestrator   |->| SSD Runner     |
+-------^--------+  +----------------+  +----------------+  +-------+--------+
        |                                                        |
        |                                                        v
+-------+--------+     +----------------+       +--------+-------+
| Verifier       |<--->| Code Review    |<------| Agent Adapters |
| / Scorer       |     | Judge          |       +--------+-------+
+-------+--------+     +----------------+                |
        ^                                                v
        |                                        +--------+-------+
        |                                        | Sample Event   |
        |                                        | Renderer / TUI |
        |                                        +----------------+

+----------------+   +----------------+
| Runtime Access |   | Review Comment |
|                |   | Controller     |
+----------------+   +----------------+
```

This Component View decomposes the `CLI / Daemon` container only. Every component runs in the same Cyanos executable; none is a separate deployable service.

Primary relationships:

| Source | Target | Relationship |
| --- | --- | --- |
| CLI Boundary | Task Loader | Starts execution for one configured project and selected task id. |
| Task Loader | Intake Gate | Passes the selected GitHub issue as the task candidate. |
| Task Loader | GitHub Issues | Reads the selected issue body, comments, and accessibility state. |
| Intake Gate | GitHub Issues | Posts blocker comments and waits for clarification resolution. |
| Intake Gate | Orchestrator | Sends a clear, frozen task brief into the evolution loop. |
| Orchestrator | SSD Runner | Starts each inner Sample-Select-Distill run with the current prompt snapshot and sample seeding policy. |
| SSD Runner | Agent Adapters | Launches isolated sample attempts in separate worktrees. |
| Agent Adapters | Coding Agent CLIs | Invokes concrete agent processes behind the abstraction boundary. |
| Agent Adapters | Sample Event Renderer / TUI | Emits per-sample tool activity events. With `--samples > 1`, TTY output renders N columns; with one sample or non-TTY output, events are line-oriented. |
| SSD Runner | Verifier / Scorer | Sends sample candidates for deterministic verification, configured recall capture, code-review input, and scoring. |
| Verifier / Scorer | Code Review Judge | Supplies the task brief, selected diff, relevant repository context, and verifier evidence to an independent LLM-as-judge. |
| Code Review Judge | Verifier / Scorer | Returns structured verdict, rubric, risk, next action, and findings; it performs no GitHub mutation and has only minimal read/search/inspect tools. |
| Verifier / Scorer | Eval Result Builder | Normalizes structural checks, recall categories, code-review category penalties, and findings into `eval.json`. |
| Eval Result Builder | Prompt Evolver | Sends the selected best eval result as the only evidence for the next coding prompt. |
| Verifier / Scorer | Orchestrator | Returns accepted or rejected eval evidence for state transition decisions. |
| Runtime Access | Runtime Store | Reads and writes task briefs, prompts, ledgers, samples, and eval results. |
| Orchestrator | Target Git Repository | Promotes accepted best source state and commits `cyanos/<task-id>` and `feature/<task-id>` branches. |
| Orchestrator | Review Comment Controller | Requests PR comment ingestion, Cyanos-owned comment posting, and resolution of fixed or outdated review threads. |
| Review Comment Controller | GitHub PR / CI | Opens or updates PR comments, reads external and Cyanos-created review comments, resolves threads, and never delegates those mutations to the judge. |
| Orchestrator | GitHub PR / CI | Opens or updates PRs, watches checks, converts failures into feedback, and stops at `pr_ready`. |

| Component | Responsibility |
| --- | --- |
| CLI Boundary | Parses `init` and `run`, renders user-facing output, and routes commands. |
| Task Loader | Loads the configured GitHub repository and selected issue id. |
| Intake Gate | Blocks unclear requirements with issue comments before evolution starts. |
| Orchestrator | Owns the task state machine, run scheduling, prompt-evolution cadence, best promotion, and PR delivery lifecycle. |
| SSD Runner | Runs isolated samples, preserves lineages, and selects the best candidate. |
| Prompt Evolver | Updates runtime `prompt.<N>.md` snapshots from the selected best eval result. |
| Verifier / Scorer | Runs structural gates, configured recall categories, code-review scoring, category penalties, and promotion/readiness decisions. |
| Code Review Judge | Reviews selected diffs with minimal tools and returns structured findings only; it cannot mutate GitHub, runtime state, or repository source. |
| Eval Result Builder | Converts verifier and scoring evidence into the canonical outer-loop feedback artifact. |
| Adapters | Invokes concrete coding agent CLIs through the model-agnostic boundary. |
| Sample Event Renderer / TUI | Routes each sample's tool events to the correct column or line output as `[{tool_name}: {summary}]`. |
| Review Comment Controller | Normalizes all PR review comments into findings, posts Cyanos comments with provenance markers, and resolves fixed or outdated threads. |
| Runtime Access | Provides the filesystem boundary to task briefs, prompts, ledgers, sample metadata, eval results, patches, and checkpoint evidence. |

## Evolution Loop Mechanics

The loop has two clocks. The exploration clock compares code candidates within one run. The learning clock updates the coding prompt once after the selected candidate has been evaluated.

```text
Outer prompt-evolution loop

              +----------------------+
              | prompt.r.md          |
              +----------+-----------+
                         |
                         v
              +----------+-----------+
              | inner SSD run        |
              +----------+-----------+
                         |
                         v
              +----------+-----------+
              | selected eval.json   |
              +----------+-----------+
                         |
                         v
              +----------+-----------+
              | gate decision        |
              +----------+-----------+
                         |
               +---------+---------+
               |                   |
               v                   v
      +--------+---------+  +------+-----------+
      | update PR        |  | write prompt.r+1 |
      +------------------+  +------+-----------+
                                |
                                v
                         +------+-------+
                         | next run     |
                         +------+-------+
                                |
                                v
                         inner SSD run
```

```text
Inner Sample-Select-Distill loop for run r

+----------------+   +----------------+   +----------------+
| sample 1       |   | sample 2       |   | sample N       |
+-------+--------+   +-------+--------+   +-------+--------+
        |                    |                    |
        v                    v                    v
+-------+--------+   +-------+--------+   +-------+--------+
| eval.json      |   | eval.json      |   | eval.json      |
+-------+--------+   +-------+--------+   +-------+--------+
        |                    |                    |
        +--------------------+--------------------+
                             |
                             v
                   +---------+----------+
                   | select best        |
                   +---------+----------+
                             |
                             v
                   return to outer loop
```

The inner loop is parallel and disposable at the worktree level; each sample can fail without corrupting other samples. The outer loop is serial and stateful; it decides whether the current prompt produced useful behavior and writes exactly one next prompt snapshot.

### Run State Contract

| Step | Owner | Reads | Writes | Purpose |
| --- | --- | --- | --- | --- |
| Freeze brief | Intake Gate | GitHub issue and resolved comments | `tasks/<task-id>/README.md` | Makes the requirement stable before agent execution. |
| Start run | Orchestrator | `prompt.<r-1>.md`, `ledger.jsonl`, global best metadata | `prompt.<r>.md` | Establishes the prompt all samples in the run must share. |
| Plan samples | SSD Runner | Task shape, lineage policy, global best metadata | sample execution roots | Decides whether each sample continues its lineage or starts from elite preseed. |
| Execute sample | Agent Adapters | Prompt snapshot, frozen brief, sample execution root | `summary.md`, `patch.diff`, sample event stream | Produces one independent code candidate through a concrete CLI and records a human-readable sample summary. |
| Evaluate sample | Verifier / Scorer | Sample checkout, diff, and task verifier contract | `score.json` | Applies structural gates and computes comparable score evidence. |
| Normalize feedback | Eval Result Builder | Verifier findings, recall results, structured judge output, score | `eval.json` | Produces the canonical outer-loop feedback payload. |
| Select best | SSD Runner | All sample `eval.json` and `score.json` files | selected sample marker in `ledger.jsonl` | Chooses the candidate to compare against global best. |
| Promote best | Orchestrator | selected sample patch, eval evidence, and materialized checkout | `best.json`, `cyanos/<task-id>` commit, `result.json` | Advances the authoritative source state only when evidence improves and updates task-level progress. |
| Evolve prompt | Prompt Evolver | selected best `eval.json`, `program.md`, previous prompt | next `prompt.<r+1>.md` | Changes the coding prompt once per run, based on verified failure or weakness. |
| Deliver PR | Orchestrator | accepted global best source state and eval evidence | `feature/<task-id>` PR | Opens or updates the user-mergeable PR and watches readiness signals. |

`eval.json` is the boundary between verification and prompt evolution. Raw verifier details may be referenced by it, but prompt revision must not infer from ad hoc logs or `summary.md`. It reads the normalized verdict, failure class, findings, configured recall summaries, structured code-review findings, and prompt-evolution hints from the selected sample's eval result.

### State Transitions

The Orchestrator owns this state machine. A state transition requires recorded evidence in `ledger.jsonl` and, when it follows sample execution, the selected sample's `eval.json`.

```text
blocked_intake
    -> brief_frozen
    -> running_samples
    -> evaluated_samples
    -> global_best_rejected -> prompt_evolved -> running_samples
    -> global_best_accepted -> pr_opened -> pr_ready
```

The transition table is an N x N matrix: every durable task state is present as both a source row and a target column. A `Txx` entry means that source-to-target transition is allowed under the matching rule below. `hold` means the task remains in the same state while waiting for external evidence or a running subprocess; it is not a completed transition. `-` means the transition is invalid and must be rejected by the Orchestrator.

State abbreviations:

| Abbrev | State |
| --- | --- |
| BI | `blocked_intake` |
| BF | `brief_frozen` |
| RS | `running_samples` |
| ES | `evaluated_samples` |
| GR | `global_best_rejected` |
| PE | `prompt_evolved` |
| GA | `global_best_accepted` |
| PO | `pr_opened` |
| PR | `pr_ready` |

| From / To | BI | BF | RS | ES | GR | PE | GA | PO | PR |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| BI | hold | T01 | - | - | - | - | - | - | - |
| BF | T02 | hold | T03 | - | - | - | - | - | - |
| RS | - | - | hold | T04 | - | - | - | - | - |
| ES | T05 | - | - | hold | T06 | - | T09 | - | - |
| GR | - | - | - | - | hold | T07 | - | - | - |
| PE | - | - | T08 | - | - | hold | - | - | - |
| GA | - | - | - | - | - | - | hold | T10 | - |
| PO | T13 | - | T12 | - | - | - | - | hold | T11 |
| PR | T15 | - | T14 | - | - | - | - | - | terminal |

| ID | From | To | Transition trigger | Required evidence | Owner action |
| --- | --- | --- | --- | --- | --- |
| T01 | `blocked_intake` | `brief_frozen` | All blocker comments are answered or explicitly resolved. | GitHub issue comments and intake decision. | Write the resolved task brief to `tasks/<task-id>/README.md`. |
| T02 | `brief_frozen` | `blocked_intake` | The frozen brief is found incomplete before execution starts. | Intake finding and blocker comment draft. | Post blocker comments, discard the invalid brief snapshot, and wait for clarification. |
| T03 | `brief_frozen` | `running_samples` | Project lock, dependency checks, prompt snapshot, and sample plan are ready. | Dependency report, `prompt.<r>.md`, sample plan. | Start one SSD run with N isolated sample worktrees. |
| T04 | `running_samples` | `evaluated_samples` | Every sample exits, times out, or is cancelled under policy. | Sample execution roots, exit status, `summary.md`, `patch.diff`. | Run structural checks, configured recall checks, code-review judge, and eval normalization. |
| T05 | `evaluated_samples` | `blocked_intake` | Evaluation proves the requirement is still ambiguous or blocked by missing user context. | Selected `eval.json`, failure class, and blocker comment draft. | Post blocker comments and pause evolution until clarification arrives. |
| T06 | `evaluated_samples` | `global_best_rejected` | No sample improves global best or hard gates fail. | `eval.json`, `score.json`, `result.json`, selection record. | Record the rejected hypothesis and failure class in the task result. |
| T07 | `global_best_rejected` | `prompt_evolved` | Prompt Evolver writes the next prompt snapshot. | Selected `eval.json`, previous prompt, `prompt.<r+1>.md`. | Record the prompt-evolution decision and next prompt snapshot. |
| T08 | `prompt_evolved` | `running_samples` | The next run is scheduled from the evolved prompt. | `prompt.<r+1>.md`, dependency report, sample plan. | Start the next SSD run with N isolated sample worktrees. |
| T09 | `evaluated_samples` | `global_best_accepted` | Selected sample improves global best and passes hard gates. | Selected patch, `eval.json`, `score.json`, promotion decision. | Materialize the selected source state, commit `cyanos/<task-id>`, and write `best.json`. |
| T10 | `global_best_accepted` | `pr_opened` | PR branch is prepared from accepted global best. | `feature/<task-id>` commit, PR body evidence. | Open or update the PR from `feature/<task-id>`. |
| T11 | `pr_opened` | `pr_ready` | Required workflow checks, required metadata gates, code-review findings, and configured recall categories pass with no unresolved review concerns. | Workflow results, coverage or other configured recall evidence, review thread state. | Push the `pr_ready` runtime checkpoint and stop without merging. |
| T12 | `pr_opened` | `running_samples` | CI, configured recall evidence, metadata, or review feedback fails readiness. | Failed check logs, review comments, or eval failure summary. | Convert the failure into a new eval finding and resume evolution. |
| T13 | `pr_opened` | `blocked_intake` | PR feedback reveals missing requirement context rather than an implementation defect. | Review comment, issue update, and blocker comment draft. | Post blocker comments, pause PR readiness work, and wait for clarification. |
| T14 | `pr_ready` | `running_samples` | New review feedback, CI rerun failure, or changed requirement invalidates readiness but remains executable. | Review comment, failed check, or updated issue requirement. | Re-enter evolution with the new failure evidence. |
| T15 | `pr_ready` | `blocked_intake` | New review feedback or issue discussion reveals missing requirement context. | Review comment or issue update plus blocker comment draft. | Post blocker comments and wait for clarification before resuming evolution. |

`pr_ready` is the terminal Cyanos-owned MVP state. A later user merge is an external GitHub event outside the Cyanos run lifecycle; Cyanos must not merge the PR or require a merge before reporting the PR ready.

## Key Decisions

### Agent Boundary

All concrete agent behavior lives behind adapters. The adapter owns command construction, supported model defaults, authentication checks, and process invocation. The orchestration layer passes typed turns to `AgentRuntime`; it does not import adapter modules, inspect concrete agent names, or choose model-specific behavior.

This boundary is enforced by lint. Direct references to concrete agents outside `src/agent` and direct selection objects inside loop or eval code are policy violations.

### Sample Activity Output

Agent stream events cross a Cyanos-owned sample-event boundary before reaching the terminal. Adapters normalize each tool call into `{sample, tool_name, summary, status}` events; they do not write directly to shared terminal UI state.

When `--samples > 1` and stdout is a TTY, the Sample Event Renderer owns the multi-column TUI. It renders exactly one bordered column per sample, colors the title and border by sample status, and routes every tool call to the column for that sample. Tool-call rows use `[{tool_name}: {summary}]`.

When `--samples 1` or stdout is not a TTY, Cyanos does not start the TUI. It writes line-oriented sample activity to stdout using the same `[{tool_name}: {summary}]` content; non-TTY multi-sample output also includes the sample identity so log consumers can route events.

### Requirement Intake Gate

The intake gate must not send an unclear or unreadable requirement into the evolution loop. When an issue lacks required context, acceptance criteria, verification details, or scope boundaries, Cyanos posts blocker comments on the issue and records the task as blocked. The same block happens when any required section contains more than 5 bullet items without grouping those bullets under subheadings; the blocker comment names the section and asks for grouped subheadings before samples can start.

This is not interactive agent steering. The user does not continue a CLI session, choose samples, approve prompt changes, or answer questions inside the evolution loop. Clarification is asynchronous product intake on the source issue: Cyanos waits for answers and resumes the blocked task only after every blocker is answered or explicitly resolved.

At that point Cyanos writes a frozen task brief to the runtime directory. The evolution loop reads that brief, not the raw issue discussion, so each run has a stable requirement source. If later evidence shows the brief is still ambiguous, Cyanos records that as eval feedback and moves the task back to blocked intake rather than asking the user mid-run.

### Task Identity

For GitHub-based projects, the Cyanos task id is the GitHub issue id. Issue `123` maps to `tasks/123/`, `cyanos/123`, and `feature/123`. This keeps runtime state, evolution history, PR branches, and issue discussion aligned under one stable identifier.

Cyanos must not generate task ids for projects. Non-GitHub task sources must provide their own durable task id before Cyanos can start evolution.

### Two-Layer Evolution

Cyanos separates exploration from learning. The inner Sample-Select-Distill loop explores N code candidates under one shared prompt snapshot. The outer prompt-evolution loop learns from the selected best eval result and changes the next prompt snapshot once.

This prevents two failure modes: sample-to-sample prompt drift inside one run, and prompt changes based on unselected or low-quality attempts. The concrete execution contract is defined in Evolution Loop Mechanics.

### Lineage Strategy

Task shape controls sample memory:

- Bugfix tasks are treated as unimodal. Cyanos keeps N persistent lineages so each sample can continue its own path across runs.
- Feature tasks are treated as multimodal. Cyanos uses lineages plus elite preseed; by default sample 1 starts from global best and samples 2..N continue their own lineages.

Lineage preserves the evolution trail. Selection promotes a source state; it does not erase losing attempts.

A lineage is a logical sample identity, not a separate runtime directory. The durable history for sample `m` is the ordered sequence `runs/1/samples/<m>/`, `runs/2/samples/<m>/`, and so on, plus the matching entries in `ledger.jsonl`.

For bugfix tasks, every sample can continue from its previous completed sample commit or patch evidence. For feature tasks, sample 1 can be preseeded by materializing the commit identified in `best.json`, while samples 2..N continue from their own prior completed sample evidence. This gives multimodal exploration without requiring every historical worktree to remain on disk.

### Runtime Ownership

Cyanos can operate on any repository, so runtime artifacts must not pollute the target repository. Task ledgers, prompts, transcripts, eval results, configured recall output, and sample metadata stay under `~/.cyanos/projects/<project-id>/tasks/<task-id>/`, where `<project-id>` is the path-safe id derived from `--project <repo-locator>`.

The managed runtime directory is itself a versioned Git repository. `cyanos init` derives the default runtime repository from the target repository as `<owner>/cyanos-<repo>` and persists it in project config; `runtime.repository = "owner/custom-runtime"` may override that value. Runtime checkpoint commits and tags are pushed only to this runtime repository, never to the target source repository.

The target repository stores source code history only. `cyanos/<task-id>` records promoted evolution states. `feature/<task-id>` contains the final PR candidate. Cyanos must not write setup files into the user's repository; user-controlled repository changes happen through task branches and PRs.

For GitHub issue tasks, `<task-id>` is the issue id.

### Verification and Scoring

The MVP score is multiplicative and evidence-based:

```text
composite = structural * recall
structural = passed_structural_gates / total_structural_gates
recall = product(recall_item_scores)
```

Binary pass/fail verifier signals belong in `structural`: repository changed, compile passed, lint passed, tests passed, and other hard gates. Normalized measures belong in `recall`: coverage, requirement recall, configured benchmark deltas, and code-review categories. Code-review categories start at `1.0`; valid structured findings lower their category scores through fixed severity penalties, and code-review recall is the product of those category scores. The MVP score model must not use hand-tuned weights, raw comment counts, free-form judge scores, or heuristic multipliers; failure class is feedback for prompt evolution, not an override that dominates the numeric score.

Each sample's `eval.json` is the normalized verifier output for that sample. `tasks/<task-id>/result.json` is the task-level score, status, and verifier-feedback source. It stores baseline, current, best, and perfect score breakdowns. Each breakdown includes the total composite score and the components used to compute it: quality tier, structural gate ratio, normalized recall items, and final multiplicative score.

The task result also stores one record per outer run:

| Field | Meaning |
| --- | --- |
| `run_index` | Outer run index. |
| `selected_sample` | Sample selected by SSD for this run. |
| `previous_score` | Score breakdown before this run. |
| `selected_score` | Score breakdown for the selected sample. |
| `delta` | Selected total score minus previous total score. |
| `status` | `baseline`, `improved`, `regressed`, or `perfect`. |
| `feedback.eval_path` | Path to the selected sample's canonical `eval.json`. |
| `feedback.summary` | Verifier summary for this run. |
| `feedback.failure_class` | Failure class used by the outer prompt-evolution loop. |
| `feedback.findings` | Actionable verifier findings. |
| `feedback.recall_summary` | Coverage, requirement recall, code-review category, and configured benchmark/E2E summaries or reasons they were not applicable. |
| `feedback.next_action` | Orchestrator decision: block, revise prompt, continue, promote, update PR, or wait for PR readiness. |

The outer prompt evolver reads verifier feedback from the selected run record in `result.json` and may follow its `eval_path` for details. It must not infer state from `summary.md` or raw logs.

### Code Review Verification Ownership

Code-review verification is an independent LLM-as-judge boundary, not a generic score supplied by the coding agent and not a GitHub automation actor. Cyanos provides the judge with the frozen task brief, selected diff, relevant repository context, and verifier evidence. The judge runs with the smallest practical tool surface: read, search, and inspect capabilities only, with no repository mutation, external task execution, GitHub mutation, MCP server loading, or user/project skills.

The judge returns structured review evidence: verdict, rubric version, risk category, next action, and findings. Every finding must include category, severity, concern, concrete fix suggestion, and evidence reference. Missing fix suggestions or malformed findings are invalid judge output and block promotion or PR readiness.

Cyanos owns all PR comment operations. It posts Cyanos-created judge feedback with stable provenance markers, reads Cyanos and external review comments, normalizes every valid comment into the same finding model, decides whether evidence proves a thread fixed or outdated, resolves fixed threads, and feeds remaining findings back into scoring and prompt evolution. Comment provenance is metadata; it never removes a valid concern from lifecycle handling or code-review recall.

Task result status has four values:

| Status | Meaning |
| --- | --- |
| `baseline` | Current score is equal to the task baseline. |
| `improved` | Current score is above the task baseline but below perfect. |
| `regressed` | Latest selected run scored below the previous task score. |
| `perfect` | Current score reached the configured perfect threshold and all hard gates passed. |

A task is not ready when a PR is merely opened. After evolution starts, Cyanos's only goal is to make the PR fully implement the user's intent so the user can merge it without comments or concerns. Cyanos must not merge the PR itself. The MVP run reaches `pr_ready` only after required GitHub workflow checks pass, required metadata is present, the repository coverage gate is satisfied when configured for the target project, at least one real recall category is recorded, code-review categories have no blocking findings, CI test and coverage evidence is attached to the PR when available, and review comments and concerns are resolved.

## Runtime Contract

Runtime paths are stable and intentionally inspectable:

```text
~/.cyanos/projects/<project-id>/
|-- origin/
|-- cyanos.lock
`-- tasks/
    `-- <task-id>/
        |-- README.md
        |-- program.md
        |-- prompt.md
        |-- prompt.0.md
        |-- prompt.1.md
        |-- ledger.jsonl
        |-- result.json
        |-- best.json
        `-- runs/
            `-- 1/
                `-- samples/
                    |-- 1/
                    |   |-- summary.md
                    |   |-- patch.diff
                    |   |-- score.json
                    |   `-- eval.json
                    `-- 2/
                        |-- summary.md
                        |-- patch.diff
                        |-- score.json
                        `-- eval.json
```

| Path | Responsibility |
| --- | --- |
| `origin/` | Cyanos-managed clone of the target repository used to create branches and worktrees. |
| `cyanos.lock` | Project-level lock; only one Cyanos process may operate on a project at a time. |
| `tasks/<task-id>/README.md` | Frozen task brief generated from the resolved requirement. |
| `program.md` | Runtime copy of the prompt-revision task prompt. |
| `prompt.md` | Initial coding task prompt copied from the executable. |
| `prompt.<run>.md` | Immutable coding prompt snapshot used by one run. |
| `ledger.jsonl` | Append-only decision log linking runs, samples, scores, statuses, and promotion commits. |
| `result.json` | Task-level result state with score breakdowns, run deltas, selected verifier feedback, status, PR readiness, and next action. |
| `best.json` | Metadata for the accepted best, including selected run/sample, score, `eval.json`, `patch.diff`, target commit, `cyanos/<task-id>` commit, and PR branch commit identity. |
| `runs/<run>/samples/<sample>/worktree/` | Optional temporary Git worktree for one independent agent attempt; it is materialized for execution or diagnosis and may be discarded after durable evidence is written. |
| `summary.md` | Human-readable sample summary for audit and debugging only; it is not a scoring, promotion, or prompt-evolution input. |
| `patch.diff` | Source diff produced by that sample against its starting state. |
| `score.json` | Composite scoring inputs and final score details for that sample. |
| `eval.json` | Normalized eval result derived from structural verifiers, recall categories, and structured code-review output; this is the feedback artifact consumed by the outer prompt-evolution loop. |

Run and sample directories use plain decimal identifiers: `runs/1/` and `samples/1/`. Durable resume and audit state comes from `ledger.jsonl`, `result.json`, `best.json`, `patch.diff`, `score.json`, `eval.json`, runtime checkpoint commits/tags, and promoted target-repository commits. Worktrees are execution materializations, not durable truth; Cyanos may discard incomplete or stale worktrees after preserving the evidence needed to resume safely. Lineage is reconstructed from the ordered sample records and `ledger.jsonl`; there is no separate `lineages/` directory.

The verifier may link to `summary.md` for diagnostics, but pass/fail decisions must come from the materialized checkout, diff, command results, configured recall evidence, and structured code-review output normalized into `eval.json`.

## Branch Contract

Each task owns two branches:

```text
main
|-- cyanos/<task-id>   # promoted evolution history
`-- feature/<task-id>   # final PR candidate
```

Global best promotion materializes the accepted source state, commits it to `cyanos/<task-id>`, and records the promoted commit in `best.json` and `result.json`. Finalization prepares `feature/<task-id>` from the accepted source state, opens or updates the PR from that branch, and keeps the task active only until `pr_ready`.

## Release Model

The supported installation path is Homebrew only:

```text
brew install johnsonlee/tap/cyanos
```

Releases are created by pushing `v*` tags. GitHub Actions builds executable artifacts, publishes the GitHub Release, and updates `johnsonlee/homebrew-tap`. Source builds are not the user distribution path.
