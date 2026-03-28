# AGENTS.md

Operational guidance for coding agents in this repository.

## 1) Project Context
- Project: ddak — terminal agent orchestrator (Rust workspace).
- Primary architecture source: `PROJECT_SPEC.md`.
- Canonical work tracking: `ddak` issues in local state file `.ddak/tickets.duckdb`.
- Canonical domain work item: `issue`.

## 2) Environment
- This repo standardizes on `devenv` + `direnv`.
- `.envrc` requires `devenv`; shell entry fails if missing.
- Rust toolchain is pinned in `rust-toolchain.toml`.
- Workspace edition is Rust `2024`.

Bootstrap:
```bash
nix profile install nixpkgs#devenv
direnv allow
devenv shell
```

## 3) Repository Layout
- `Cargo.toml` (workspace members + shared deps)
- `crates/` (Rust crates)
- `PROJECT_SPEC.md` (product and architecture contract)
- `Makefile` (command aliases)
- `docs/work-tracking.md` (CLI work tracking workflow)

## 4) Build, Lint, and Test Commands
Run commands from repo root.

Core commands:
- `cargo check --workspace`
- `cargo fmt --all`
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Makefile aliases:
- `make check`
- `make fmt`
- `make clippy`
- `make test`

Run one crate:
- `cargo check -p orchestrator-core`
- `cargo test -p orchestrator-core`

Run a single test (important):
- Substring match: `cargo test -p orchestrator-core session_state_transitions`
- Exact unit test: `cargo test -p orchestrator-core --lib session_state_transitions -- --exact`
- Single integration test target: `cargo test -p orchestrator-core --test fsm_transitions`
- Show captured output: append `-- --nocapture`

Run app binary:
- `cargo run -p tui-app`

## 5) Required Validation Before Completion
Run these unless explicitly blocked:
1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`

If a check is skipped, document why and what was verified instead.

## 6) Code Style Guidelines

Formatting:
- Follow rustfmt output; do not fight formatter.
- Prefer readable multi-line formatting for long chains.
- No trailing whitespace.

Imports:
- Group in order: `std`, external crates, internal modules.
- Keep groups stable and alphabetized where practical.
- Avoid glob imports (`use foo::*`) unless justified.

Naming:
- Types/traits/enums: `UpperCamelCase`.
- Functions/modules/variables: `snake_case`.
- Constants/statics: `UPPER_SNAKE_CASE`.
- Boolean fields/methods: predicate style (`is_*`, `has_*`, `can_*`).
- Canonical status identifiers are snake_case: `backlog`, `ready`, `in_progress`, `review`, `done`, `blocked`.

Types and API design:
- Prefer explicit structs/enums for domain boundaries.
- Avoid untyped maps for core protocol/state paths.
- Keep public APIs narrow; do not expose internals early.
- Prefer borrowing when ownership is unnecessary (`&str`, slices).

Error handling:
- Library crates: typed errors via `thiserror`.
- Binary/orchestration boundaries: `anyhow` is acceptable.
- Do not swallow errors; propagate with context.
- Return actionable error messages and preserve cause chains.

Logging/telemetry:
- Use `tracing` for structured diagnostics.
- Include `session_id` / correlation metadata when available.
- Never log API keys, tokens, or raw secrets.

Comments/docs:
- Add comments for non-obvious logic only.
- Prefer clear naming over explanatory comments.
- Add concise docs on public APIs with non-trivial behavior.

## 7) Architecture Guardrails (Spec-Derived)
- Embedded PTY interactive runtime is first-class.
- Do not redesign around headless-only execution.
- DuckDB is durable state and replay store, not live message bus.
- Event processing must be idempotent and sequence-aware.
- Preserve `issue_id` <-> `session_id` linkage invariants.
- Keep transport progression compatible: in-process -> stdio -> HTTP.
- Integration adapters translate external schemas into canonical internal model.

## 8) Testing Expectations
- Add unit tests near logic-heavy modules.
- Add integration tests for runtime lifecycle and reconciliation paths.
- For bug fixes, prefer test-first regression coverage.
- Keep tests deterministic; avoid brittle timing assertions.
- For adapter behavior, cover parse/mapping edge cases.

## 9) Workflow and Ticket Conventions
- Ticket IDs use `DDAK-XXX`.
- Keep work scoped to a ticket where feasible.
- Use `ddak issue move` to reflect status transitions.
- Keep `PROJECT_SPEC.md` and active issue comments aligned with behavior changes.

## 10) Cursor/Copilot Rule Files
Checked locations:
- `.cursor/rules/`
- `.cursorrules`
- `.github/copilot-instructions.md`

Current state:
- No Cursor rule files found.
- No Copilot instruction file found.
- This `AGENTS.md` is the authoritative agent instruction file in-repo.

## 11) Practical Agent Defaults
- Prefer small, composable changes over broad refactors.
- Verify by running commands, not by assumption.
- Keep docs/spec and `ddak` issue comments updated when contracts change.
- If guidance conflicts, follow `PROJECT_SPEC.md` and this file.

## 12) Additional Rule Files
- Shared repository rules live under `rules/`.
- Git commit message policy is defined in `rules/git-commit-messages.md`.
- When this file and a rule file overlap, follow the stricter rule.
