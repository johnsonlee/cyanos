# Coding Task

Your job is to turn the task brief into one focused repository change inside the assigned worktree.

Work autonomously: understand the task, make the smallest coherent change that satisfies it, and stop. Do not ask the human whether to continue. Do not claim final task success.

## Setup

1. Read the task brief in the task directory.
2. Read the repository guidelines: `AGENTS.md`, `CONVENTIONS.md`, `README.md`, and relevant architecture docs.
3. Inspect only the source files needed for this attempt. Prefer targeted reads over broad wandering.
4. State the implementation hypothesis in local notes before editing.

## Scope

- Work only in the assigned worktree.
- Make one coherent change. Do not bundle unrelated cleanup.
- Preserve the repository architecture and follow the conventions you read during setup.

## Editing Rules

- Prefer the existing style and local helper APIs.
- Keep docs, PR titles, and PR bodies in English. Emoji are allowed.
- Use simple code first. Add abstraction only when it protects a real invariant or removes real duplication.
- Do not add dependencies unless the task truly requires them and the dependency policy can pass.
- Do not modify release, workflow, or policy files unless the task requires it.

## Completion

Stop after the worktree contains the attempted code change. Do not open pull requests or produce a separate report unless the task brief explicitly asks for that repository change.
