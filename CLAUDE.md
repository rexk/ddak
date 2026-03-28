# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Authoritative References

- `AGENTS.md` — primary agent operational guidance (read first for full detail)
- `PROJECT_SPEC.md` — product and architecture contract
- `rules/git-commit-messages.md` — commit message policy

If guidance conflicts, follow the stricter requirement.

## Project

ddak — a terminal agent orchestrator. Rust workspace for a terminal-first orchestrator that manages multiple interactive AI agent sessions (OpenCode, Claude Code) with Kanban-style issue tracking, DuckDB persistence, and optional Linear integration.

## Environment

- Requires `devenv` + `direnv` (Nix-based). Run `direnv allow` then `devenv shell`.
- Rust toolchain pinned in `rust-toolchain.toml`. Workspace edition: Rust 2024.

## Build / Lint / Test

```bash
cargo check --workspace          # or: make check
cargo fmt --all --check          # or: make fmt
cargo clippy --workspace --all-targets -- -D warnings  # or: make clippy
cargo test --workspace           # or: make test
```

Single crate: `cargo test -p orchestrator-core`
Single test: `cargo test -p orchestrator-core session_state_transitions`
Run app: `cargo run -p tui-app`

**Before completing any task**, run fmt check, clippy, and tests (all three).

## Workspace Crates

```
crates/
  orchestrator-core    — session lifecycle FSM, scheduling, task/session linking
  runtime-pty          — embedded PTY spawn/IO/resize (not tmux/zellij)
  terminal-surface     — VT parser, per-session screen grid, pane rendering
  adapter-opencode     — OpenCode agent adapter
  adapter-claudecode   — Claude Code agent adapter
  store-duckdb         — DuckDB persistence (event log + projections)
  rpc-core             — JSON-RPC style protocol definitions
  transport-stdio      — stdio transport for local client/server
  integration-linear   — Linear PM sync
  tui-app              — ratatui TUI application (entry point)
```

## Architecture Essentials

- **Embedded PTY** is the session runtime (not tmux/zellij). This is intentional — do not redesign around headless-only execution.
- **DuckDB** is durable state and replay store, not a live message bus.
- Agent adapters normalize events into a common model (`session.started`, `output.delta`, `prompt.detected`, etc.) with idempotent, sequence-aware processing.
- `issue_id` ↔ `session_id` linkage is a core invariant.
- Transport progression: in-process → stdio → HTTP. Keep compatible.
- TUI uses `ratatui`; interactive panes render from terminal-surface state, not line-oriented logs.

## Code Conventions

- Error handling: `thiserror` in library crates, `anyhow` at binary boundaries.
- Logging: `tracing` with structured diagnostics and `session_id` correlation.
- Avoid glob imports (`use foo::*`).
- Boolean fields: predicate style (`is_*`, `has_*`, `can_*`).

## Commit Messages

Format: `<type>: <short summary>` (imperative mood, ≤72 chars). Types: `feat`, `fix`, `refactor`, `test`, `docs`, `build`, `chore`. Reference ticket IDs (`DDAK-XXX`) when applicable.

## Verifying Rendering Changes

After modifying `terminal-surface`, `session_bus`, or `board_poc` rendering:

```bash
cargo test -p terminal-surface                         # VT parsing + visual snapshots
cargo test -p tui-app --test render_pipeline            # full PTY→screen pipeline
cargo test -p tui-app --test visual_snapshots           # PTY-based snapshot tests
```

If snapshot diffs appear, follow `rules/rendering-verification.md` to interpret and accept/reject. Use `INSTA_UPDATE=always` (not `cargo insta review`).

Helpers in `orchestrator_core::session_bus`:
- `wait_for_screen_content()` — poll-based screen predicate with timeout
- `screen_dump_with_attrs()` — debug dump of vt100 screen state

## Work Tracking

- Ticket IDs: `DDAK-XXX`, tracked in `.ddak/tickets.duckdb`.
- CLI: `ddak issue` commands for status transitions.
- Canonical statuses: `backlog`, `ready`, `in_progress`, `review`, `done`, `blocked`.
