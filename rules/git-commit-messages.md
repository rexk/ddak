# Git Commit Message Rules

Use these rules for all commits in this repository.

## Required Format

- Preferred subject format: `<type>: <short summary>`
- Subject line target length: <= 72 characters
- Use imperative mood ("add", "fix", "refactor", not "added", "fixes")

Recommended commit types:

- `feat`
- `fix`
- `refactor`
- `test`
- `docs`
- `build`
- `chore`

## Ticket Reference

- Reference ticket IDs when applicable (`DDAK-XXX`).
- Ticket IDs may be in subject or body.

Examples:

- `feat: add PTY session lifecycle state machine (DDAK-005)`
- `fix: prevent out-of-order session event replay (DDAK-003)`
- `docs: clarify stdio-first transport rollout`

## Message Body Guidance

- Explain why the change is needed, not only what changed.
- Add notable constraints, migration notes, or trade-offs.
- Keep lines readable (target <= 100 chars per line).

## Scope and Hygiene

- Keep each commit focused on a single logical change.
- Avoid mixing unrelated refactors with behavior changes.
- Do not commit secrets, local environment artifacts, or generated files unless explicitly intended.

## Agent Notes

- If a commit closes or advances a ticket, mention status impact in PR/summary text.
- If commit hooks fail, fix issues and create a new commit message that still follows this policy.
