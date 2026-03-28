# ddak — Terminal Agent Orchestrator - Project Specification (v0.1)

## 1. Overview

This project is a terminal-first orchestrator for managing multiple AI agent sessions and their project/task progress in one place.

The system is designed to:

- run natively in the terminal via a TUI,
- manage many concurrent agent sessions without losing interactivity,
- provide a Kanban-style project view,
- persist state locally in DuckDB,
- integrate with external PM tools such as Linear,
- work with both OpenCode and Claude Code through a common adapter protocol,
- expose a common client/server protocol over both stdio and HTTP.

The initial architecture uses **embedded PTY session management** (Option A), not tmux/zellij as the core runtime dependency.
The system will adopt a zellij-like internal terminal-surface architecture for pane fidelity while keeping orchestrator state and APIs independent from zellij.

---

## 2. Goals and Non-Goals

### 2.1 Goals (v1)

1. Launch and manage multiple interactive agent sessions from one TUI.
2. Keep each session interactive (streaming input/output, interruption, resizing).
3. Track work in a Kanban model linked to sessions, tasks, and projects.
4. Persist operational data in local DuckDB.
5. Provide optional sync/integration with Linear.
6. Support OpenCode and Claude Code via adapters and a normalized event model.
7. Provide a transport-neutral RPC interface over:
   - local stdio, and
   - HTTP (optionally remote).
8. Run reliably on Linux and macOS.

### 2.2 Non-Goals (v1)

1. Building a general-purpose terminal multiplexer.
2. Exposing or persisting orchestrator sessions as zellij sessions.
3. Running a mandatory HTTPS server locally.
4. Full cloud orchestration platform in v1.
5. Windows support in v1.

---

## 3. Key Product Principles

1. **Terminal-native first**: all critical workflows available in TUI and CLI.
2. **Local-first state**: DuckDB as source of truth for projects/sessions/tasks.
3. **Adapter-driven**: support multiple agent tools via well-defined adapter contracts.
4. **Transport-agnostic API**: same protocol methods over stdio and HTTP.
5. **Composable deployment**: run as local app, local daemon, or remote service.

### 3.1 Canonical Domain Naming (Linear/Jira-Aligned)

Use industry-standard issue tracking terms in the core model.

- `workspace`
- `team`
- `project`
- `issue`
- `status`
- `priority`
- `assignee`
- `labels`
- `comment`

Notes:

- The UI may still render a board as cards, but the canonical record type is `issue`.
- Integrations map provider-specific terms (Linear `state`, Jira `status`) to canonical `status`.

---

## 4. Core Architecture (Option A)

### 4.1 High-Level Components

1. **TUI Application**
   - Main operator UI
   - Session grid/list/detail views
   - Kanban board and workflow actions
   - Event/log viewer
   - Built with `ratatui` for layout and interaction widgets

2. **Orchestrator Core**
   - Session lifecycle state machine
   - Scheduling and command routing
   - Task/session linking
   - Permission and policy checks

3. **Session Runtime (Embedded PTY Engine)**
   - Spawn agent processes directly via PTY
   - Bi-directional stream handling
   - Resize, interrupt, terminate, restart
   - Structured event emission

4. **Terminal Surface Layer (Embedded VT Pane Engine)**
   - Parse PTY bytes with VT parser/state machine
   - Maintain per-session screen grid (cursor, attributes, scrollback, alt-screen)
   - Render pane viewport diffs into TUI frame
   - Route pane-local input and resize events

5. **Agent Adapters**
   - OpenCode adapter
   - Claude Code adapter
   - Normalization of events/status/capabilities

6. **Persistence Layer (DuckDB)**
   - Event log
   - Snapshot/projection tables
   - Query APIs for UI and integrations

7. **Integration Connectors**
   - Linear sync (initial external target)
   - future connectors (GitHub Projects, Jira, etc.)

8. **RPC Interface**
   - JSON-RPC style API
   - two transports: stdio and HTTP

### 4.2 Why Embedded PTY First

- Avoids tight coupling/conflict with tmux/zellij when running inside tmux/zellij.
- Keeps behavior consistent across nested terminal contexts.
- Preserves full control over session state model and event stream.
- Allows tmux/zellij adapters to be added later as optional backends.

### 4.3 TUI + Terminal Surface Composition Contract

1. `ratatui` owns board/workflow/session-metadata UI composition.
2. Interactive agent panes are rendered from terminal-surface state, not line-oriented logs.
3. Session pane fidelity requirements include cursor addressing, alternate screen, SGR attributes, and resize propagation.
4. Sanitized line fallback may be used only for non-interactive log views.
5. Orchestrator identity and persistence remain canonical (`issue_id`, `session_id`) and are never delegated to zellij session metadata.

---

## 5. Runtime and Process Model

### 5.1 Runtime Modes

1. **Standalone mode (default)**
   - TUI + orchestrator in one process.

2. **Client/server local mode**
   - Daemon process + one or more TUI clients.

3. **Remote mode (optional)**
   - Client connects to remote orchestrator via HTTP.

### 5.2 Session Lifecycle

States:

- `created`
- `starting`
- `running`
- `awaiting_input`
- `busy`
- `suspended`
- `completed`
- `failed`
- `terminated`

Transitions are event-driven and persisted.

### 5.3 Session Control Actions

- create session
- attach/detach viewer
- send input
- resize
- interrupt
- restart
- terminate
- capture output excerpt
- annotate status

### 5.4 Interactive Session Identity and Notification Model

This system is designed for interactive terminal sessions (not headless job execution).

#### Identity model

1. Each issue has a canonical local `issue_id`.
2. Each live agent terminal gets an orchestrator `session_id`.
3. `issue_session_links` stores the relation (`issue_id` <-> `session_id`) with lifecycle metadata.
4. Adapter/runtime-specific IDs are stored as external handles, for example:
   - `adapter_session_ref` (provider-native session key when available),
   - `runtime_pid`/pty handle metadata,
   - `resume_hint` command (when native ID is not exposed).

#### Working rule (v1 default)

- One active interactive session per `in_progress` issue by default.
- Additional sessions per issue are allowed but marked as secondary.

#### Notification sources

1. PTY stream events (`output.delta`, process exit, resize, interrupts).
2. Adapter-derived state hints (`awaiting_input`, `busy`, `error`, prompt-detected).
3. Ticket workflow events (move to blocked/done, relink, handoff).

#### Notification fanout (local-first)

1. Active pane: full real-time stream.
2. Inactive sessions: summarized activity counters and last-event timestamp.
3. Board view: badge indicators (`active`, `needs_input`, `error`, `done`).
4. Optional daemon mode: same events fanned out to attached clients.

#### Embedded terminal pane requirements

1. PTY output MUST be interpreted as terminal control stream, not plain text.
2. Pane rendering MUST derive from terminal screen state and preserve in-pane interaction semantics.
3. Input routing MUST support raw key sequences, paste, control keys, and terminal mode toggles required by interactive tools.
4. Runtime MUST terminate or reconcile orphaned pane processes on shutdown/restart.

#### Resume behavior

1. If adapter exposes resumable session identity, restore by `adapter_session_ref`.
2. If not, persist `runtime_instance_id` and restore by orchestrator metadata + `resume_hint`.
3. Resume without adapter-native identity uses a confidence score; low-confidence reattach requires explicit operator confirmation.
4. If process is gone, keep transcript/state and mark session `terminated` with restart action.

---

## 6. Common Adapter Protocol (Agent Bridge)

### 6.1 Adapter Contract

Each adapter implements:

1. `probe()`
   - Detect tool availability and version.

2. `start_session(config)`
   - Spawn process and return runtime handles.

3. `write_input(session_id, bytes_or_text)`
4. `resize(session_id, cols, rows)`
5. `interrupt(session_id)`
6. `terminate(session_id)`
7. `read_events(session_id)`
   - Stream normalized events.

### 6.2 Normalized Event Model

Event types:

- `session.started`
- `session.stopped`
- `output.delta`
- `output.flush`
- `prompt.detected`
- `task.detected`
- `status.changed`
- `error`

Event envelope requirements:

- `event_id` (globally unique)
- `session_id`
- `session_seq` (monotonic per session)
- `correlation_id`
- `emitted_at` (UTC)
- `schema_version`

Consumer guarantees:

- Consumers MUST be idempotent on `event_id`.
- Consumers MUST enforce monotonic `session_seq` or mark gaps for reconciliation.

### 6.3 Adapter Capability Flags

- `supports_interruption`
- `supports_resume`
- `supports_structured_tasks`
- `supports_cost_metrics`
- `supports_model_switch`

---

## 7. Kanban and Project Model

### 7.1 Core Concepts

- **Workspace**: top-level scope
- **Team**: execution and ownership scope inside a workspace
- **Project**: collection of issues/sessions
- **Issue**: canonical unit of work tracked on board/list views
- **Session Link**: one or many sessions attached to an issue

### 7.2 Default Kanban Columns

1. `Backlog`
2. `Ready`
3. `In Progress`
4. `Review`
5. `Done`
6. `Blocked`

### 7.3 Automation Rules (v1)

- Creating a session from an issue moves issue to `in_progress`.
- Session failure can auto-tag issue as `blocked`.
- Manual completion moves issue to `done`.
- Optional confirmation gates for auto-move behavior.

### 7.4 Canonical Issue Fields

Minimum canonical issue schema (aligned with Linear/Jira conventions):

- `id`
- `identifier` (human-readable key, similar to Jira key / Linear identifier)
- `title`
- `description`
- `status`
- `priority`
- `project_id`
- `team_id`
- `assignee_id`
- `labels`
- `estimate`
- `due_date`
- `created_at`
- `updated_at`
- `completed_at`

---

## 8. DuckDB Data Model

### 8.1 Design Approach

- Append-only event table for auditability.
- Projection tables for fast TUI queries.
- Deterministic rebuild of projections from event log.
- Projection rebuild emits a checksum to validate deterministic recovery.
- Startup recovery reconciles persisted `running` sessions against live processes/PTY handles.
- Orphaned processes are reattached when possible, otherwise marked `terminated_orphaned` with reason code.

### 8.2 Initial Tables

1. `workspaces`
2. `teams`
3. `projects`
4. `boards`
5. `board_columns`
6. `issues`
7. `sessions`
8. `session_events`
9. `issue_session_links`
10. `integrations`
11. `integration_mappings`
12. `sync_state`

### 8.3 Required Indexing

- `sessions(project_id, status)`
- `session_events(session_id, ts)`
- `issues(board_id, column_id, position)`
- `integration_mappings(external_id)`

---

## 9. RPC Interface and Transports

### 9.1 Protocol Style

- JSON-RPC inspired request/response + notifications.
- Stable method namespace and typed payload schemas.

### 9.2 Base Method Groups

1. `session.*`
   - `session.create`
   - `session.list`
   - `session.get`
   - `session.input`
   - `session.resize`
   - `session.interrupt`
   - `session.terminate`
   - `session.subscribe`

2. `board.*`
   - `board.list`
   - `board.issue.create`
   - `board.issue.move`
   - `board.issue.update`
   - `board.issue.link_session`

3. `issue.*`
   - `issue.create`
   - `issue.list`
   - `issue.get`
   - `issue.update`
   - `issue.comment.add`

4. `project.*`
   - `project.create`
   - `project.list`
   - `project.get`

5. `integration.*`
   - `integration.connect`
   - `integration.sync.pull`
   - `integration.sync.push`
   - `integration.mapping.upsert`

6. `system.*`
   - `system.health`
   - `system.version`
   - `system.capabilities`

### 9.3 Transport Requirements

1. **stdio transport**
   - local process communication
   - low overhead

2. **HTTP transport**
   - local or remote client/server
   - optional auth and TLS for remote use

### 9.4 HTTPS Position

- Not required for local v1.
- Supported as optional hardening for remote deployment.

### 9.5 Client/Server Topology Options

This project supports three practical topologies. They are complementary, not mutually exclusive.

1. **Fat client (no daemon, direct local runtime)**
   - TUI process directly owns:
     - PTY runtime,
     - DuckDB connection,
     - adapter lifecycle.
   - No HTTP server required.
   - No local RPC required unless plugins/extensions need it.

2. **Local daemon + stdio client protocol**
   - A local daemon owns runtime and state.
   - TUI runs as a client and talks over stdio (spawned child) or local IPC wrapper.
   - Good for multi-window attach and process isolation.

3. **Daemon + HTTP protocol**
   - Same core RPC methods exposed over HTTP.
   - Enables remote clients, automation, and multi-machine workflows.
   - Auth/TLS requirements increase operational complexity.

### 9.6 Decision for v1 and v1.1

1. **v1 default: Fat client mode**
   - Fastest path.
   - Best terminal-native ergonomics.
   - Zero local service management burden.

2. **v1 optional: stdio protocol mode (local daemon)**
   - Implement same RPC surface in-process first.
   - Expose it via stdio transport for local client/server split.

3. **v1.1+: HTTP mode**
   - Add once stdio RPC semantics are stable.
   - Keep method and payload compatibility with stdio.

### 9.7 Filesystem-Only Coordination (Explicit Position)

A pure filesystem protocol (eg, only lock files + append logs + polling) is **not** the primary *interactive* control plane.

Allowed usage:

- DuckDB file as durable state store.
- Export/import snapshots.
- Crash recovery metadata.

Not recommended as primary RPC substitute:

- request/response control flow over files,
- high-frequency session streaming via file polling,
- multi-client real-time coordination through file watchers alone.

Reason: this introduces race conditions, poor latency semantics, and brittle cross-platform watcher behavior under high event volume.

Important clarification:

- DuckDB remains the authoritative durable state store.
- The concern is specifically about using DB/file polling as the real-time notification bus for interactive session I/O.

### 9.7.1 Practical Scope of RPC (v1)

The system does **not** assume heavy remote RPC traffic in v1.

1. Most operations are local and terminal-native.
2. Session execution and high-frequency output handling stay client-side.
3. Remote HTTP mode, when enabled, is primarily for state sync/coordination and optional remote visibility.
4. Until remote execution is introduced, server-side scope is intentionally narrow.

### 9.7.2 Notification Strategy (v1)

Notification focus is local first:

1. In-process event fanout for UI panes/views.
2. Optional local daemon fanout for multiple terminal clients.
3. Persist all significant events to DuckDB for replay/recovery.
4. Use transport notifications for live UX; use DuckDB for truth/history.

### 9.8 Protocol Requirements (Applies to Stdio and HTTP)

1. Request/response correlation via `id`.
2. Out-of-band notifications for streaming events.
3. Backpressure-aware event delivery.
4. Idempotency keys for mutating calls (`session.create`, `issue.create`, `board.issue.create`, etc.).
5. Capability negotiation at connect time.
6. Versioned schema envelope for forward compatibility.
7. Mutating calls MUST include optimistic concurrency version for target entities.
8. In daemon mode, session control uses per-`session_id` single-writer action queues.
9. Streaming output uses bounded buffers and emits `output.dropped` telemetry on overflow.

### 9.9 Recommended Rollout Sequence

1. Build core APIs in-process (fat client).
2. Bind those APIs to stdio transport without changing method contracts.
3. Add HTTP transport reusing the same handlers and schemas.
4. Add auth/TLS only for remote deployments.

### 9.10 Session Launch CWD Policy

Session launch CWD is deterministic and absolute-path only.

Resolution order (highest precedence first):

1. Runtime CWD override (operator/runtime scoped).
2. Issue CWD override.
3. Project local repository path.

Rules:

1. Launch CWD candidates MUST be absolute paths.
2. Launch CWD candidates MUST exist and be directories.
3. If no valid CWD resolves, launch MUST be blocked with actionable operator error.
4. TUI process CWD is not an implicit fallback for issue-linked launches.
5. Effective resolved CWD should be visible for operator diagnostics and replay/audit context.

---

## 10. tmux/zellij Compatibility Strategy

The app is expected to run well when launched inside tmux/zellij while using embedded PTYs internally.

### 10.1 Rules

1. Do not require controlling parent tmux/zellij session.
2. Use alternate screen responsibly.
3. Respect terminal resize and pass through to child PTYs.
4. Provide optional "external pane launch" integration later, not in core v1.
5. Runtime MUST restore terminal state (echo/canonical/alt-screen) on panic/crash/forced exit paths.
6. Compatibility validation includes nested tmux/zellij, rapid resize storms, bracketed paste, and interrupt propagation.
7. Compatibility work does not alter canonical orchestrator session storage or identity model.

### 10.2 Future Optional Adapters

- `backend-tmux`
- `backend-zellij`

These are additive and disabled by default.

---

## 11. Linear Integration (v1)

### 11.0 Integration Architecture Principle

External PM integrations are layered behind an internal canonical model.

1. The product owns a canonical `Project` + `Issue` domain schema.
2. Integrations attach links/mappings to canonical records.
3. We do not reshape the core schema to mirror each external provider.
4. Provider-specific behavior lives in provider adapters and mapping config.

### 11.1 Scope

1. Link project to Linear team/project.
2. Map local issue <-> Linear issue.
3. Pull issue updates.
4. Push state/title/description updates (configurable).

### 11.2 Conflict Strategy

- Default: local edits win until explicit sync action.
- Store last sync cursor and conflict flags.
- Provide manual conflict resolution command in TUI.

### 11.3 Canonical-to-External Mapping Model

#### Canonical status set (v1)

- `backlog`
- `ready`
- `in_progress`
- `review`
- `done`
- `blocked`

#### Mapping storage

Mappings are stored as integration configuration, scoped per workspace/project:

- `integration_type` (eg, `linear`)
- `workspace_id`
- `project_id`
- `external_project_id`
- `status_map` (canonical -> external)
- `reverse_status_map` (external -> canonical)
- `field_map` (title, description, assignee, labels, priority, etc.)
- `sync_policy` (pull-only, push-only, bidirectional)
- `mapping_version`
- `last_validated_at`

#### Mapping behavior

1. Canonical remains source model for local UX and automation rules.
2. Sync layer translates canonical events to provider API calls.
3. Provider webhook/pull updates are translated back into canonical updates.
4. Unknown external states/fields enter a quarantine state and pause push sync for affected issues until remapped.

### 11.4 Sync Engine Responsibilities

1. Translate canonical records to provider payloads.
2. Maintain `integration_mappings` for object identity.
3. Persist sync cursors/checkpoints for incremental sync.
4. Enforce configurable status mapping rules.
5. Detect and flag mapping errors (unknown state, missing field mapping, permissions).
6. Keep sync side effects idempotent.

### 11.5 Configuration UX (v1)

1. TUI wizard to connect provider.
2. Select external team/project.
3. Configure status map interactively.
4. Save mapping profile in DuckDB.
5. Dry-run sync preview before enabling bidirectional mode.

---

## 12. Security and Safety

1. Local-first defaults.
2. Explicit opt-in for remote HTTP mode.
3. Token storage via OS keychain where available; fallback encrypted file.
4. Command execution restricted to configured adapter commands.
5. Full action/event audit trail in DuckDB.
6. Remote HTTP mode requires authentication and TLS by default.
7. Any insecure remote mode requires explicit unsafe flag and startup warning.
8. Fallback secret files require per-user encryption keys, 0600 permissions, and token rotation/revocation support.

---

## 13. Recommended Implementation Stack

### 13.1 Language Choice for v1

**Rust recommended** for v1 due to mature ecosystem for:

- TUI (`ratatui`, `crossterm`)
- PTY (`portable-pty`)
- terminal parser/surface (`vte`/`vt100`-class crates)
- local DB (`duckdb-rs`)
- async runtime/tooling

Zig remains a valid future path, especially for low-level terminal rendering/control specialization.

### 13.2 Suggested Modules

1. `orchestrator-core`
2. `runtime-pty`
3. `adapter-opencode`
4. `adapter-claudecode`
5. `store-duckdb`
6. `rpc-core`
7. `transport-stdio`
8. `transport-http`
9. `integration-linear`
10. `tui-app`

---

## 14. Milestones

### M0 - Foundation

- repository bootstrap
- config model
- DuckDB schema v0
- basic TUI shell

### M1 - Session Runtime MVP

- embedded PTY spawn/input/output/resize
- session list/detail views
- terminal-surface viewport rendering for interactive agent panes
- basic persistence for sessions and events

### M2 - Agent Adapters

- OpenCode adapter
- Claude Code adapter
- normalized event pipeline

### M3 - Kanban Core

- board model + issue CRUD/move
- issue-session linking
- workflow shortcuts in TUI

### M4 - RPC and Daemon Mode

- stdio transport
- subscriptions/event streaming
- per-session action serialization and concurrency conflict handling

### M5 - Linear Integration

- OAuth/API key setup
- issue mapping and sync
- conflict handling UI

### M6 - Hardening

- stress testing for many concurrent sessions
- crash recovery and replay
- packaging and docs
- tmux/zellij nested compatibility test matrix
- overflow/backpressure telemetry and diagnostics bundle export
- terminal-surface correctness checks (alternate screen, cursor addressing, resize churn)

### M7 - HTTP Transport (v1.1+)

- HTTP transport using same RPC schemas
- auth/TLS hardening for remote mode

---

## 15. Open Questions for Next Research Pass

1. Best prompt/status detection heuristics per adapter.
2. Multi-client concurrency control semantics in daemon mode.
3. Event streaming protocol details over HTTP (SSE vs websocket vs long-poll).
4. Local encryption key strategy cross-platform.
5. Migration/versioning strategy for DuckDB schema evolution.

---

## 16. Acceptance Criteria for v1

1. Manage at least 10 concurrent agent sessions interactively.
2. p95 input-to-echo latency under 150ms in local mode.
3. Kanban operations remain responsive under active output streams.
4. State survives restart with deterministic recovery checksum match.
5. OpenCode and Claude Code sessions both supported through adapters.
6. Same core API available in-process and over stdio transport.
7. Linear sync operational for mapped projects/issues.
8. No lost control actions under two concurrent clients attached to one session in daemon mode.
9. OpenCode renders and remains interactive inside embedded session pane without terminal corruption.
10. Session shutdown/restart paths do not leave unmanaged orphan agent processes under normal exit conditions.

---

## 17. Future Extensions (Post-v1)

1. Optional tmux/zellij backends.
2. Distributed worker execution.
3. Team multi-user collaboration and RBAC.
4. Rich analytics (cycle time, throughput, model/runtime costs).
5. LLM-assisted project manager workflows.
