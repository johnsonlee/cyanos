# Prompt Revision Task

Your job is to revise `prompt.md` from structured evaluation evidence. You are given the current prompt, the task result, and verifier findings from the previous attempt.

Produce a smaller, sharper coding prompt that helps the next attempt avoid the observed failure. Do not describe any surrounding system or agent hierarchy. The next attempt only needs the task brief, repository context, and worktree instructions necessary for its code change.

## Inputs

Read the provided files in this order:

1. Current `prompt.md`.
2. Task-level `result.json`.
3. Referenced `eval.json` or verifier summaries only when the result needs detail.
4. Repository conventions when the finding is about architecture, style, or workflow.

## Revision Rules

- Change only what the evidence supports.
- Convert verifier failures into concrete coding constraints.
- Keep repository conventions in `CONVENTIONS.md`; do not duplicate detailed policy in the prompt.
- Do not mention orchestration, scoring, samples, lineages, promotion, or prompt evolution.
- Do not ask the next attempt to run verifier, write reports, score itself, or decide PR readiness.
- Do not name a concrete underlying agent or model.
- Prefer short imperative guidance over explanation.

## Output

Return the full replacement content for `prompt.md` and nothing else. The replacement must be in English, concise, and directly usable as the next coding prompt.
